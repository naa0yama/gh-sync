/// CLI argument definitions for the `init` subcommand.
pub mod cli;
/// Mode A: copy the upstream's own gh-sync config.
mod copy;
/// Mode B: interactively generate a config from an upstream file listing.
mod generate;
/// JSON Schema constant and writer helper.
pub mod schema;
/// Interactive file + strategy picker widget.
mod select;

use std::io::{self, IsTerminal as _, Write as _};
use std::path::Path;
use std::process::ExitCode;

use cli::InitArgs;

use crate::sync::upstream::GhFetcher;

// ---------------------------------------------------------------------------
// Mode enum — defined at module level to avoid `items_after_statements`
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum Mode {
    FromUpstream,
    Select,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return `true` when `repo` matches the `owner/name` pattern.
fn is_valid_repo(repo: &str) -> bool {
    let Some((owner, name)) = repo.split_once('/') else {
        return false;
    };
    if name.contains('/') {
        return false;
    }
    let valid_segment = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    };
    valid_segment(owner) && valid_segment(name)
}

/// Validate repo format and return a descriptive error if invalid.
///
/// # Errors
///
/// Returns an error when `repo` does not match the `owner/name` pattern.
fn validate_repo_format(repo: &str) -> anyhow::Result<()> {
    if is_valid_repo(repo) {
        Ok(())
    } else {
        anyhow::bail!(
            "invalid repository '{repo}': must be owner/name format \
             (e.g. naa0yama/boilerplate-rust)"
        )
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Execute the `init` subcommand.
///
/// Writes a new gh-sync config file (and `schema.json`) to the
/// output path, creating parent directories as needed.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn execute(args: &InitArgs) -> ExitCode {
    match run(args, &GhFetcher) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // {e:#} prints the full error chain (context: cause: root cause)
            tracing::error!("init failed: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Core logic for `init`, parameterised over the upstream fetcher for
/// testability.
///
/// # Errors
///
/// Returns an error when:
/// - The output file already exists and the user declines to overwrite it.
/// - The repo cannot be determined (no `--repo` flag, no TTY).
/// - Upstream fetching fails.
/// - The output file cannot be written.
#[allow(clippy::too_many_lines)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn run(
    args: &InitArgs,
    fetcher: &dyn crate::sync::upstream::UpstreamFetcher,
) -> anyhow::Result<()> {
    use anyhow::Context as _;

    // -----------------------------------------------------------------------
    // 1. Check for existing output file
    // -----------------------------------------------------------------------
    if args.output.exists() && !args.force {
        if io::stdin().is_terminal() {
            let confirmed = dialoguer::Confirm::new()
                .with_prompt(format!(
                    "'{}' already exists. Overwrite?",
                    args.output.display()
                ))
                .default(false)
                .interact()
                .context("confirmation prompt cancelled")?;
            if !confirmed {
                let mut stdout = io::stdout();
                writeln!(stdout, "Aborted.").context("failed to write to stdout")?;
                return Ok(());
            }
        } else {
            anyhow::bail!(
                "'{}' already exists; use --force to overwrite",
                args.output.display()
            );
        }
    }

    // -----------------------------------------------------------------------
    // 2. Determine repo
    // -----------------------------------------------------------------------
    let repo = match &args.repo {
        Some(r) => {
            validate_repo_format(r)?;
            r.clone()
        }
        None => {
            if io::stdin().is_terminal() {
                dialoguer::Input::<String>::new()
                    .with_prompt("Upstream repository (owner/name)")
                    .validate_with(|input: &String| -> Result<(), &str> {
                        if is_valid_repo(input) {
                            Ok(())
                        } else {
                            Err("must be owner/name format (e.g. naa0yama/boilerplate-rust)")
                        }
                    })
                    .interact_text()
                    .context("repo prompt cancelled")?
            } else {
                anyhow::bail!(
                    "--repo is required in non-interactive mode\n\
                     example: gh-sync init --repo owner/name --from-upstream"
                );
            }
        }
    };

    // -----------------------------------------------------------------------
    // 3. Determine mode
    // -----------------------------------------------------------------------
    let mode = if args.from_upstream {
        Mode::FromUpstream
    } else if args.select {
        Mode::Select
    } else if io::stdin().is_terminal() {
        let choices = [
            "Copy upstream's gh-sync config",
            "Select files interactively",
        ];
        let idx = dialoguer::Select::new()
            .with_prompt("How would you like to create the config?")
            .items(&choices)
            .default(0)
            .interact()
            .context("mode selection cancelled")?;
        if idx == 0 {
            Mode::FromUpstream
        } else {
            Mode::Select
        }
    } else {
        anyhow::bail!(
            "no mode specified; use --from-upstream or --select\n\
             example: gh-sync init --repo owner/name --from-upstream"
        );
    };

    // -----------------------------------------------------------------------
    // 4. Generate config content
    // -----------------------------------------------------------------------
    let content = match mode {
        Mode::FromUpstream => copy::fetch_upstream_config(fetcher, &repo, &args.ref_)?,
        Mode::Select => generate::run_interactive(fetcher, &repo, &args.ref_, "")?,
    };

    // -----------------------------------------------------------------------
    // 5. Write output file and schema.json
    // -----------------------------------------------------------------------
    let output_dir = args.output.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create directory '{}'", output_dir.display()))?;

    std::fs::write(&args.output, &content)
        .with_context(|| format!("failed to write '{}'", args.output.display()))?;

    schema::write_schema_file(output_dir)
        .with_context(|| format!("failed to write schema.json to '{}'", output_dir.display()))?;

    let mut stdout = io::stdout();
    writeln!(stdout, "[OK] created '{}'", args.output.display())
        .context("failed to write to stdout")?;
    writeln!(
        stdout,
        "[OK] created '{}/schema.json'",
        output_dir.display()
    )
    .context("failed to write to stdout")?;

    Ok(())
}
