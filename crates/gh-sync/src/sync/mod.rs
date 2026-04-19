/// CLI argument definitions for the `sync` subcommand
pub mod cli;
/// Auto-detection of the fork / template parent repository
pub mod detect;
/// Unified diff generation via the `diff` CLI
pub mod diff;
/// Error types for manifest validation and sync operations
pub mod error;
/// Manifest schema, loading, and validation
pub mod manifest;
/// Synchronisation modes (validate, sync, ci-check, patch-refresh)
pub mod mode;
/// Formatted output helpers
pub mod output;
/// Repository spec sync via the `gh` CLI
pub mod repo;
/// Abstracts spawning the `gh` CLI for mock injection in tests
pub mod runner;
/// Synchronisation strategies (`replace`, `create_only`, `delete`, `patch`)
pub mod strategy;
/// Upstream file fetching via the `gh` CLI
pub mod upstream;
/// Parsing and fetching of `--upstream-manifest` references
pub mod upstream_manifest;

use std::io::{self, IsTerminal as _};
use std::process::ExitCode;

use crate::sync::cli::SyncCommand;
use crate::sync::runner::SystemGhRunner;
use crate::sync::strategy::patch::RealPatchRunner;
use crate::sync::upstream::GhFetcher;

/// Execute the `sync` subcommand.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn execute(args: &cli::SyncArgs) -> ExitCode {
    match &args.command {
        SyncCommand::File(file_args) => execute_file(file_args),
        SyncCommand::Repo(repo_args) => repo::execute(repo_args),
    }
}

/// Execute `sync file` — fetch upstream files, preview, then apply on confirmation.
#[cfg_attr(coverage_nightly, coverage(off))]
#[allow(clippy::too_many_lines)]
fn execute_file(args: &cli::SyncFileArgs) -> ExitCode {
    let mut stdout = io::stdout();

    // Resolve the directory containing the manifest as the repo root.
    let repo_root = args
        .manifest
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .unwrap_or_else(|| std::path::Path::new("."));

    // --validate without --upstream-manifest: use the fast file-based path.
    if args.validate && args.upstream_manifest.is_none() {
        return match mode::validate::run(&args.manifest, repo_root, &mut stdout) {
            Ok(code) => code,
            Err(e) => {
                tracing::error!("sync file --validate I/O error: {e}");
                ExitCode::FAILURE
            }
        };
    }

    // Detect fork/template parent and ask the user if they want to use it as
    // the upstream manifest, unless --upstream-manifest is already specified.
    let runner = SystemGhRunner;
    let effective_upstream = detect::resolve_effective_upstream(
        args.upstream_manifest.as_deref(),
        args.yes || args.ci_check || args.dry_run || args.patch_refresh,
        &args.manifest,
        &runner,
    );

    // Resolve the effective manifest (upstream fetch + optional local overlay,
    // or just local file when --upstream-manifest is not given).
    let fetcher = GhFetcher;
    let manifest =
        match upstream_manifest::resolve(effective_upstream.as_deref(), &args.manifest, &fetcher) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("failed to resolve manifest: {e:#}");
                tracing::info!("hint: run `gh-sync init` to create a configuration file");
                return ExitCode::FAILURE;
            }
        };

    // --validate with --upstream-manifest: validate the merged manifest.
    if args.validate {
        return match mode::validate::run_manifest(&manifest, repo_root, &mut stdout) {
            Ok(code) => code,
            Err(e) => {
                tracing::error!("sync file --validate I/O error: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if let Err(e) = manifest::validate_schema(&manifest) {
        tracing::error!("manifest validation failed: {e}");
        return ExitCode::FAILURE;
    }

    let patch_runner = RealPatchRunner;

    if args.ci_check {
        return match mode::ci_check::run(&manifest, repo_root, &fetcher, &patch_runner, &mut stdout)
        {
            Ok(code) => code,
            Err(e) => {
                tracing::error!("sync file --ci-check I/O error: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if args.patch_refresh {
        return match mode::patch_refresh::run(&manifest, repo_root, &fetcher, &mut stdout) {
            Ok(code) => code,
            Err(e) => {
                tracing::error!("sync file --patch-refresh I/O error: {e}");
                ExitCode::FAILURE
            }
        };
    }

    // Default: preview all changes, then optionally apply.
    let (preview_code, actions) =
        match mode::sync::run(&manifest, repo_root, &fetcher, &patch_runner, &mut stdout) {
            Ok(result) => result,
            Err(e) => {
                tracing::error!("sync file I/O error: {e}");
                return ExitCode::FAILURE;
            }
        };

    // When the preview itself detected errors (conflicts, fetch failures),
    // return immediately — there is nothing safe to apply.
    if preview_code != ExitCode::SUCCESS {
        return preview_code;
    }

    // Nothing to apply.
    if actions.is_empty() {
        return ExitCode::SUCCESS;
    }

    // --dry-run: show preview only, do not prompt.
    if args.dry_run {
        return ExitCode::SUCCESS;
    }

    // --yes: apply without prompting.
    if args.yes {
        return apply_file(actions);
    }

    // Interactive: ask the user for confirmation.
    if io::stdin().is_terminal() {
        let confirmed = dialoguer::Confirm::new()
            .with_prompt("Apply these changes?")
            .default(false)
            .interact()
            .unwrap_or(false);

        if confirmed {
            return apply_file(actions);
        }

        tracing::info!("aborted — no changes were written");
        return ExitCode::SUCCESS;
    }

    // Non-interactive without --yes: refuse to apply.
    tracing::error!(
        "changes detected but stdin is not a TTY; use --yes to apply or --dry-run to suppress this error"
    );
    ExitCode::FAILURE
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn apply_file(actions: Vec<mode::sync::ApplyAction>) -> ExitCode {
    if let Err(e) = mode::sync::apply_outcomes(actions) {
        tracing::error!("failed to apply changes: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
