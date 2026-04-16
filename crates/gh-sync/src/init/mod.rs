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
/// GitHub Actions workflow template generator.
pub mod workflow;

use std::io::{self, IsTerminal as _, Write as _};
use std::path::Path;
use std::process::ExitCode;

use anyhow::Context as _;
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

/// Check whether `path` needs to be (over)written with `new_content`.
///
/// - If the file does not exist, returns `Ok(true)` (safe to write).
/// - If `force` is set, returns `Ok(true)` (skip all checks).
/// - If the file exists and its content equals `new_content`, prints an
///   "already up to date" message and returns `Ok(false)`.
/// - If not a TTY, returns an error suggesting `--force`.
/// - Otherwise, shows a unified diff and prompts the user interactively.
///
/// # Errors
///
/// Returns an error when reading the existing file fails, the prompt is
/// cancelled, or the terminal is non-interactive without `--force`.
fn confirm_overwrite_with_diff(
    path: &Path,
    new_content: &str,
    force: bool,
) -> anyhow::Result<bool> {
    if !path.exists() {
        return Ok(true);
    }
    if force {
        return Ok(true);
    }

    let existing = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read '{}'", path.display()))?;

    if existing == new_content {
        let mut stdout = io::stdout();
        writeln!(stdout, "'{}' is already up to date.", path.display())
            .context("failed to write to stdout")?;
        return Ok(false);
    }

    if !io::stdin().is_terminal() {
        anyhow::bail!(
            "'{}' already exists; use --force to overwrite",
            path.display()
        );
    }

    // Display a unified diff between the existing and new content.
    let diff = similar::TextDiff::from_lines(existing.as_str(), new_content);
    let mut stdout = io::stdout();
    writeln!(
        stdout,
        "--- {} (existing)\n+++ {} (new)",
        path.display(),
        path.display()
    )
    .context("failed to write to stdout")?;
    for group in diff.grouped_ops(3) {
        for op in &group {
            for change in diff.iter_changes(op) {
                let line = change.value().trim_end_matches('\n');
                let styled = match change.tag() {
                    similar::ChangeTag::Delete => {
                        console::style(format!("-{line}")).red().to_string()
                    }
                    similar::ChangeTag::Insert => {
                        console::style(format!("+{line}")).green().to_string()
                    }
                    similar::ChangeTag::Equal => format!(" {line}"),
                };
                writeln!(stdout, "{styled}").context("failed to write to stdout")?;
            }
        }
    }

    let confirmed = dialoguer::Confirm::new()
        .with_prompt(format!("Overwrite '{}'?", path.display()))
        .default(false)
        .interact()
        .context("confirmation prompt cancelled")?;

    Ok(confirmed)
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
    // Dispatch to workflow-only mode when --with-workflow is set.
    if args.with_workflow {
        return run_workflow_only(args, fetcher);
    }

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

    // -----------------------------------------------------------------------
    // 6. Always generate the GitHub Actions workflow file
    // -----------------------------------------------------------------------
    let workflow_path = Path::new(workflow::WORKFLOW_PATH);
    let version = concat!("v", env!("CARGO_PKG_VERSION"));

    let sha = fetcher
        .resolve_tag_sha("naa0yama/gh-sync", version)
        .with_context(|| format!("failed to resolve SHA for naa0yama/gh-sync@{version}"))?;

    let upstream_manifest = format!("{repo}@main:.github/gh-sync/config.yaml");
    let rendered = workflow::render(version, &sha, Some(&upstream_manifest));

    if confirm_overwrite_with_diff(workflow_path, &rendered, args.force)? {
        workflow::write_workflow_from_content(workflow_path, &rendered)?;
        writeln!(stdout, "[OK] created '{}'", workflow_path.display())
            .context("failed to write to stdout")?;
    }

    Ok(())
}

/// Workflow-only mode: generate (or update) the GitHub Actions workflow file
/// without touching config or schema.
///
/// The `upstream-manifest` input is populated from the existing config file
/// when present; otherwise the workflow is rendered with a commented-out
/// placeholder.
///
/// # Errors
///
/// Returns an error when the SHA cannot be resolved, the workflow file cannot
/// be written, or the user declines to overwrite an existing file.
#[cfg_attr(coverage_nightly, coverage(off))]
fn run_workflow_only(
    args: &InitArgs,
    fetcher: &dyn crate::sync::upstream::UpstreamFetcher,
) -> anyhow::Result<()> {
    let version = concat!("v", env!("CARGO_PKG_VERSION"));

    // Resolve the release tag to a commit SHA.
    let sha = fetcher
        .resolve_tag_sha("naa0yama/gh-sync", version)
        .with_context(|| format!("failed to resolve SHA for naa0yama/gh-sync@{version}"))?;

    // Attempt to read upstream info from the existing config file.
    let config_path = Path::new(".github/gh-sync/config.yaml");
    let upstream_manifest = read_upstream_manifest(config_path);

    let rendered = workflow::render(version, &sha, upstream_manifest.as_deref());

    let workflow_path = Path::new(workflow::WORKFLOW_PATH);
    if confirm_overwrite_with_diff(workflow_path, &rendered, args.force)? {
        workflow::write_workflow_from_content(workflow_path, &rendered)?;
        let mut stdout = io::stdout();
        writeln!(stdout, "[OK] created '{}'", workflow_path.display())
            .context("failed to write to stdout")?;
    }

    Ok(())
}

/// Try to read `upstream-manifest` from an existing config file.
///
/// Returns `Some("owner/repo@ref:.github/gh-sync/config.yaml")` when the
/// config file exists and can be parsed; `None` otherwise.
fn read_upstream_manifest(config_path: &Path) -> Option<String> {
    let Ok(content) = std::fs::read_to_string(config_path) else {
        return None;
    };
    let Ok(manifest) = serde_yml::from_str::<gh_sync_manifest::manifest::Manifest>(&content) else {
        return None;
    };
    Some(format!(
        "{}@{}:.github/gh-sync/config.yaml",
        manifest.upstream.repo, manifest.upstream.ref_
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use tempfile::TempDir;

    use super::*;

    // ---------------------------------------------------------------------------
    // is_valid_repo
    // ---------------------------------------------------------------------------

    #[test]
    fn valid_repo_accepts_owner_slash_name() {
        assert!(is_valid_repo("owner/repo"));
        assert!(is_valid_repo("naa0yama/boilerplate-rust"));
        assert!(is_valid_repo("my.org/my_repo-name"));
    }

    #[test]
    fn valid_repo_rejects_missing_slash() {
        assert!(!is_valid_repo("no-slash"));
        assert!(!is_valid_repo(""));
    }

    #[test]
    fn valid_repo_rejects_multiple_slashes() {
        assert!(!is_valid_repo("owner/name/extra"));
    }

    #[test]
    fn valid_repo_rejects_empty_segments() {
        assert!(!is_valid_repo("/name"));
        assert!(!is_valid_repo("owner/"));
    }

    // ---------------------------------------------------------------------------
    // confirm_overwrite_with_diff
    // ---------------------------------------------------------------------------

    #[test]
    fn confirm_overwrite_returns_true_when_file_absent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.yaml");
        let result = confirm_overwrite_with_diff(&path, "new content", false).unwrap();
        assert!(result, "should return true when file does not exist");
    }

    #[test]
    fn confirm_overwrite_returns_true_with_force() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("existing.yaml");
        std::fs::write(&path, b"old content").unwrap();
        let result = confirm_overwrite_with_diff(&path, "new content", true).unwrap();
        assert!(result, "should return true when force=true");
    }

    #[test]
    fn confirm_overwrite_returns_false_when_identical() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("same.yaml");
        std::fs::write(&path, b"same content").unwrap();
        // Non-TTY is fine here because identical content exits early.
        let result = confirm_overwrite_with_diff(&path, "same content", false).unwrap();
        assert!(!result, "should return false when content is identical");
    }

    #[test]
    fn confirm_overwrite_errors_on_non_tty_with_diff() {
        // In test environment stdin is not a TTY, so differing content should error.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("changed.yaml");
        std::fs::write(&path, b"old content").unwrap();
        let result = confirm_overwrite_with_diff(&path, "new content", false);
        assert!(
            result.is_err(),
            "should error in non-TTY with differing content"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("--force"),
            "error should mention --force, got: {msg}"
        );
    }

    // ---------------------------------------------------------------------------
    // read_upstream_manifest
    // ---------------------------------------------------------------------------

    #[test]
    fn read_upstream_manifest_returns_none_for_missing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.yaml");
        assert!(read_upstream_manifest(&path).is_none());
    }

    #[test]
    #[cfg_attr(miri, ignore = "libyml (C FFI) triggers UB under Miri")]
    fn read_upstream_manifest_returns_none_for_invalid_yaml() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bad.yaml");
        std::fs::write(&path, b"not: valid: yaml: [").unwrap();
        assert!(read_upstream_manifest(&path).is_none());
    }

    #[test]
    #[cfg_attr(miri, ignore = "libyml (C FFI) triggers UB under Miri")]
    fn read_upstream_manifest_parses_valid_config() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        let yaml = "\
upstream:
  repo: owner/upstream
  ref: main
files:
  - path: README.md
    strategy: replace
";
        std::fs::write(&path, yaml).unwrap();
        let result = read_upstream_manifest(&path).unwrap();
        assert_eq!(result, "owner/upstream@main:.github/gh-sync/config.yaml");
    }
}
