/// CLI argument definitions for the `init` subcommand.
pub mod cli;
/// Interactively generate a config from an upstream file listing.
mod generate;
/// JSON Schema constant and writer helper.
pub mod schema;
/// Interactive file + strategy picker widget.
mod select;
/// Claude Code skill file generator.
mod skill;
/// GitHub Actions workflow template generator.
pub mod workflow;

use std::io::{self, IsTerminal as _, Write as _};
use std::path::Path;
use std::process::ExitCode;

use anyhow::Context as _;
use cli::InitArgs;

use crate::sync::detect;
use crate::sync::runner::{GhRunner, SystemGhRunner};
use crate::sync::upstream::GhFetcher;

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

/// Write `content` to `path`, creating parent directories as needed.
///
/// # Errors
///
/// Returns an error when directory creation or file write fails.
fn write_file(path: &Path, content: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory '{}'", parent.display()))?;
    }
    std::fs::write(path, content).with_context(|| format!("failed to write '{}'", path.display()))
}

/// Resolve the repository argument, prompting interactively when not provided.
///
/// `default_hint` is pre-filled into the interactive prompt when `--repo` is
/// absent, so the user can simply press Enter to accept the detected value.
///
/// # Errors
///
/// Returns an error when the repo format is invalid, the prompt is cancelled,
/// or `--repo` is absent in non-interactive mode.
#[cfg_attr(coverage_nightly, coverage(off))]
fn resolve_repo(
    args: &InitArgs,
    prompt: &str,
    non_tty_example: &str,
    default_hint: Option<&str>,
) -> anyhow::Result<String> {
    match &args.repo {
        Some(r) => {
            validate_repo_format(r)?;
            Ok(r.clone())
        }
        None => {
            if io::stdin().is_terminal() {
                let mut input = dialoguer::Input::<String>::new().with_prompt(prompt);
                if let Some(hint) = default_hint {
                    input = input.default(hint.to_owned()).show_default(true);
                }
                input
                    .validate_with(|i: &String| -> Result<(), &str> {
                        if is_valid_repo(i) {
                            Ok(())
                        } else {
                            Err("must be owner/name format (e.g. naa0yama/boilerplate-rust)")
                        }
                    })
                    .interact_text()
                    .context("repo prompt cancelled")
            } else {
                anyhow::bail!(
                    "--repo is required in non-interactive mode\nexample: {non_tty_example}"
                )
            }
        }
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
/// Writes files appropriate to the selected mode (upstream or downstream).
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn execute(args: &InitArgs) -> ExitCode {
    match run(args, &GhFetcher::new(), &SystemGhRunner) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("init failed: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Core logic for `init`, dispatching to upstream or downstream mode.
///
/// # Errors
///
/// Returns an error when the selected mode fails.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn run(
    args: &InitArgs,
    fetcher: &dyn crate::sync::upstream::UpstreamFetcher,
    runner: &dyn GhRunner,
) -> anyhow::Result<()> {
    if args.upstream {
        run_upstream(args, fetcher, runner)
    } else {
        run_downstream(args, fetcher, runner)
    }
}

/// Upstream mode: generate `config.yaml` + `schema.json` for a template project.
///
/// # Errors
///
/// Returns an error when the repo cannot be determined, fetching fails,
/// or the output file cannot be written.
#[cfg_attr(coverage_nightly, coverage(off))]
fn run_upstream(
    args: &InitArgs,
    fetcher: &dyn crate::sync::upstream::UpstreamFetcher,
    runner: &dyn GhRunner,
) -> anyhow::Result<()> {
    let output = args
        .output
        .as_deref()
        .unwrap_or_else(|| Path::new(".github/gh-sync/config.yaml"));

    // -----------------------------------------------------------------------
    // 1. Check for existing output file
    // -----------------------------------------------------------------------
    if output.exists() && !args.force {
        if io::stdin().is_terminal() {
            let confirmed = dialoguer::Confirm::new()
                .with_prompt(format!("'{}' already exists. Overwrite?", output.display()))
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
                output.display()
            );
        }
    }

    // -----------------------------------------------------------------------
    // 2. Determine repo (detect current repo as default hint)
    // -----------------------------------------------------------------------
    let hint = detect::detect_repo_hint(runner, false);
    let repo = resolve_repo(
        args,
        "Upstream repository (owner/name)",
        "gh-sync init --upstream --repo owner/name --select",
        hint.as_deref(),
    )?;

    // -----------------------------------------------------------------------
    // 3. Determine mode and generate config content
    // -----------------------------------------------------------------------
    let content = if args.select || io::stdin().is_terminal() {
        generate::run_interactive(fetcher, &repo, &args.ref_, "")?
    } else {
        anyhow::bail!(
            "no mode specified; use --select\n\
             example: gh-sync init --upstream --repo owner/name --select"
        );
    };

    // -----------------------------------------------------------------------
    // 4. Write output file and schema.json
    // -----------------------------------------------------------------------
    let output_dir = output.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create directory '{}'", output_dir.display()))?;

    std::fs::write(output, &content)
        .with_context(|| format!("failed to write '{}'", output.display()))?;

    let schema_outcome = schema::write_schema_file(output_dir)
        .with_context(|| format!("failed to write schema.json to '{}'", output_dir.display()))?;

    let mut stdout = io::stdout();
    writeln!(stdout, "[OK] created '{}'", output.display()).context("failed to write to stdout")?;
    match schema_outcome {
        schema::WriteOutcome::Created => writeln!(
            stdout,
            "[OK] created '{}/schema.json'",
            output_dir.display()
        )
        .context("failed to write to stdout")?,
        schema::WriteOutcome::Updated => writeln!(
            stdout,
            "[OK] updated '{}/schema.json'",
            output_dir.display()
        )
        .context("failed to write to stdout")?,
        schema::WriteOutcome::Unchanged => writeln!(
            stdout,
            "'{}/schema.json' is already up to date.",
            output_dir.display()
        )
        .context("failed to write to stdout")?,
    }

    Ok(())
}

/// Downstream mode: generate the GitHub Actions workflow file and optionally
/// a Claude Code skill file.
///
/// # Errors
///
/// Returns an error when the repo cannot be determined, the SHA cannot be
/// resolved, or file writes fail.
#[cfg_attr(coverage_nightly, coverage(off))]
fn run_downstream(
    args: &InitArgs,
    fetcher: &dyn crate::sync::upstream::UpstreamFetcher,
    runner: &dyn GhRunner,
) -> anyhow::Result<()> {
    // -----------------------------------------------------------------------
    // 1. Determine repo (detect fork/template parent as default hint)
    // -----------------------------------------------------------------------
    let hint = detect::detect_repo_hint(runner, true);
    let repo = resolve_repo(
        args,
        "Upstream template repository (owner/name)",
        "gh-sync init --downstream --repo owner/name",
        hint.as_deref(),
    )?;

    // -----------------------------------------------------------------------
    // 2. Resolve SHA and render workflow
    // -----------------------------------------------------------------------
    let version = concat!("v", env!("CARGO_PKG_VERSION"));

    let sha = fetcher
        .resolve_tag_sha("naa0yama/gh-sync", version)
        .with_context(|| format!("failed to resolve SHA for naa0yama/gh-sync@{version}"))?;

    let upstream_manifest = format!("{repo}@{}:.github/gh-sync/config.yaml", args.ref_);
    let rendered = workflow::render(version, &sha, Some(&upstream_manifest));

    // -----------------------------------------------------------------------
    // 3. Write workflow file
    // -----------------------------------------------------------------------
    let workflow_path = Path::new(workflow::WORKFLOW_PATH);
    let mut stdout = io::stdout();

    if confirm_overwrite_with_diff(workflow_path, &rendered, args.force)? {
        workflow::write_workflow_from_content(workflow_path, &rendered)?;
        writeln!(stdout, "[OK] created '{}'", workflow_path.display())
            .context("failed to write to stdout")?;
    }

    // -----------------------------------------------------------------------
    // 4. Optionally write Claude Code skill file
    // -----------------------------------------------------------------------
    if args.with_skill {
        let skill_path = Path::new(skill::SKILL_PATH);
        let skill_content = skill::render();

        if confirm_overwrite_with_diff(skill_path, &skill_content, args.force)? {
            skill::write_skill_from_content(skill_path, &skill_content)?;
            writeln!(stdout, "[OK] created '{}'", skill_path.display())
                .context("failed to write to stdout")?;
        }
    }

    Ok(())
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
}
