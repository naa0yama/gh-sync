//! Default sync mode: fetch upstream files, preview changes, then apply on confirmation.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context as _;
use gh_sync_manifest::{self as manifest, Manifest, Strategy};

use crate::diff::unified_diff;
use crate::output::{RuleOutcome, StatusTag, Summary, emit_diff, emit_status, emit_summary};
use crate::strategy::patch::PatchRunner;
use crate::strategy::{self, StrategyResult};
use crate::upstream::{FetchResult, UpstreamFetcher};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A deferred filesystem write collected during the preview phase.
///
/// Only actionable outcomes (write or delete) produce an `ApplyAction`.
/// Callers invoke [`apply_outcomes`] to commit the changes.
#[derive(Debug)]
pub enum ApplyAction {
    /// Write `content` to `local_path` (creating parent directories as needed).
    Write {
        /// Destination path on the local filesystem.
        local_path: PathBuf,
        /// Bytes to write to `local_path`.
        content: Vec<u8>,
    },
    /// Delete the file or directory at `local_path`.
    Delete {
        /// Path to remove from the local filesystem.
        local_path: PathBuf,
    },
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Preview all sync changes and emit per-rule status to `w`.
///
/// This function is **always** non-destructive: it never writes to the
/// filesystem.  To commit the changes, pass the returned [`ApplyAction`]
/// slice to [`apply_outcomes`].
///
/// Returns `(exit_code, actions)` where:
/// - `exit_code` is `FAILURE` when any rule produces a conflict or error.
/// - `actions` is the ordered list of filesystem writes to perform.
///
/// # Errors
/// Propagates I/O errors from `w`.
pub fn run(
    manifest: &Manifest,
    repo_root: &Path,
    fetcher: &dyn UpstreamFetcher,
    patch_runner: &dyn PatchRunner,
    w: &mut dyn Write,
) -> std::io::Result<(ExitCode, Vec<ApplyAction>)> {
    let mut outcomes: Vec<RuleOutcome<'_>> = Vec::new();
    let mut actions: Vec<ApplyAction> = Vec::new();

    for rule in &manifest.files {
        let local_path = repo_root.join(&rule.path);
        let local_bytes: Option<Vec<u8>> = std::fs::read(&local_path).ok();

        let result = match rule.strategy {
            Strategy::Delete => strategy::delete::apply(local_bytes.is_some()),
            Strategy::Replace | Strategy::CreateOnly => {
                let source = rule.source.as_deref().unwrap_or(&rule.path);
                match fetcher.fetch(&manifest.upstream.repo, &manifest.upstream.ref_, source) {
                    Err(e) => StrategyResult::Error(format!("upstream fetch failed: {e}")),
                    Ok(FetchResult::NotFound) => StrategyResult::Skipped {
                        reason: format!(
                            "upstream not found: {source} (use 'delete' strategy to remove local file)"
                        ),
                    },
                    Ok(FetchResult::Content(upstream)) => {
                        if rule.strategy == Strategy::Replace {
                            strategy::replace::apply(&upstream, local_bytes.as_deref())
                        } else {
                            strategy::create_only::apply(&upstream, local_bytes.is_some())
                        }
                    }
                }
            }
            Strategy::Patch => {
                let patch_path = manifest::resolve_patch_path(rule);
                let full_patch = repo_root.join(&patch_path);
                match fetcher.fetch(&manifest.upstream.repo, &manifest.upstream.ref_, &rule.path) {
                    Err(e) => StrategyResult::Error(format!("upstream fetch failed: {e}")),
                    Ok(FetchResult::NotFound) => {
                        StrategyResult::Error(format!("upstream path not found: {}", rule.path))
                    }
                    Ok(FetchResult::Content(upstream)) => strategy::patch::apply(
                        &upstream,
                        local_bytes.as_deref(),
                        &full_patch,
                        patch_runner,
                    ),
                }
            }
        };

        let diff = match &result {
            StrategyResult::Changed { content } => {
                let old = local_bytes.as_deref().unwrap_or(b"");
                unified_diff(&rule.path, old, content).unwrap_or_default()
            }
            _ => String::new(),
        };

        match &result {
            StrategyResult::Changed { content } => actions.push(ApplyAction::Write {
                local_path: local_path.clone(),
                content: content.clone(),
            }),
            StrategyResult::Deleted => actions.push(ApplyAction::Delete {
                local_path: local_path.clone(),
            }),
            _ => {}
        }

        let (tag, detail) = status_for_result(&result);
        emit_status(w, tag, &rule.path, rule.strategy, detail.as_deref())?;
        if !diff.is_empty() {
            emit_diff(w, &diff)?;
        }

        outcomes.push(RuleOutcome { rule, result, diff });
    }

    let summary = Summary::from_outcomes(&outcomes);
    emit_summary(w, &summary)?;

    let has_failure = summary.conflicts > 0 || summary.errors > 0;
    Ok((
        if has_failure {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        },
        actions,
    ))
}

/// Apply the deferred filesystem actions collected by [`run`].
///
/// Writes and deletes are performed in the order they were collected.
///
/// # Errors
/// Returns an error when any write or delete operation fails.
pub fn apply_outcomes(actions: Vec<ApplyAction>) -> anyhow::Result<()> {
    for action in actions {
        match action {
            ApplyAction::Write {
                local_path,
                content,
            } => {
                if let Some(parent) = local_path.parent() {
                    std::fs::create_dir_all(parent).with_context(|| {
                        format!("failed to create directories: {}", parent.display())
                    })?;
                }
                std::fs::write(&local_path, &content)
                    .with_context(|| format!("failed to write file: {}", local_path.display()))?;
            }
            ApplyAction::Delete { local_path } => {
                if local_path.is_dir() {
                    std::fs::remove_dir_all(&local_path).with_context(|| {
                        format!("failed to remove directory: {}", local_path.display())
                    })?;
                } else {
                    std::fs::remove_file(&local_path).with_context(|| {
                        format!("failed to remove file: {}", local_path.display())
                    })?;
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn status_for_result(result: &StrategyResult) -> (StatusTag, Option<String>) {
    match result {
        StrategyResult::Changed { .. } => (StatusTag::Changed, None),
        StrategyResult::Unchanged => (StatusTag::Ok, None),
        StrategyResult::Skipped { reason } => (StatusTag::Skipped, Some(reason.clone())),
        StrategyResult::Deleted => (StatusTag::Deleted, None),
        StrategyResult::Conflict { message } => (
            StatusTag::Warn,
            Some(format!("conflict detected — {message}")),
        ),
        StrategyResult::Error(msg) => (StatusTag::Fail, Some(msg.clone())),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::panic)]

    use std::process::ExitCode;

    use gh_sync_manifest::{Manifest, Rule, Strategy, Upstream};
    use tempfile::TempDir;

    use super::*;
    use crate::strategy::patch::testing::MockPatchRunner;
    use crate::upstream::testing::MockFetcher;

    fn make_manifest(files: Vec<Rule>) -> Manifest {
        Manifest {
            upstream: Upstream {
                repo: String::from("owner/repo"),
                ref_: String::from("main"),
            },
            spec: None,
            files,
        }
    }

    fn replace_rule(path: &str) -> Rule {
        Rule {
            path: path.to_owned(),
            strategy: Strategy::Replace,
            source: None,
            patch: None,
        }
    }

    fn delete_rule(path: &str) -> Rule {
        Rule {
            path: path.to_owned(),
            strategy: Strategy::Delete,
            source: None,
            patch: None,
        }
    }

    fn create_only_rule(path: &str) -> Rule {
        Rule {
            path: path.to_owned(),
            strategy: Strategy::CreateOnly,
            source: None,
            patch: None,
        }
    }

    // ------------------------------------------------------------------
    // replace strategy
    // ------------------------------------------------------------------

    #[test]
    fn replace_writes_new_file_when_absent() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let manifest = make_manifest(vec![replace_rule("ci.yml")]);
        let fetcher = MockFetcher::content(b"upstream content\n".to_vec());
        let runner = MockPatchRunner::success(vec![]);
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let (code, actions) = run(&manifest, dir.path(), &fetcher, &runner, &mut buf).unwrap();
        apply_outcomes(actions).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS: {out}");
        assert!(out.contains("[CHANGED]"), "missing CHANGED tag: {out}");
        let written = std::fs::read(dir.path().join("ci.yml")).unwrap();
        assert_eq!(written, b"upstream content\n");
    }

    #[test]
    fn replace_unchanged_when_content_matches() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ci.yml"), b"same content\n").unwrap();
        let manifest = make_manifest(vec![replace_rule("ci.yml")]);
        let fetcher = MockFetcher::content(b"same content\n".to_vec());
        let runner = MockPatchRunner::success(vec![]);
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let (code, _actions) = run(&manifest, dir.path(), &fetcher, &runner, &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS");
        assert!(out.contains("[OK"), "expected OK tag: {out}");
    }

    #[test]
    fn replace_skipped_when_upstream_not_found() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let manifest = make_manifest(vec![replace_rule("missing.yml")]);
        let fetcher = MockFetcher::not_found();
        let runner = MockPatchRunner::success(vec![]);
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let (code, _actions) = run(&manifest, dir.path(), &fetcher, &runner, &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS");
        assert!(out.contains("[SKIPPED]"), "expected SKIPPED: {out}");
    }

    #[test]
    fn replace_fails_when_fetch_errors() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let manifest = make_manifest(vec![replace_rule("ci.yml")]);
        let fetcher = MockFetcher::error("gh failed");
        let runner = MockPatchRunner::success(vec![]);
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let (code, _actions) = run(&manifest, dir.path(), &fetcher, &runner, &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::FAILURE), "expected FAILURE: {out}");
        assert!(out.contains("[FAIL"), "expected FAIL tag: {out}");
    }

    // ------------------------------------------------------------------
    // preview is always non-destructive
    // ------------------------------------------------------------------

    #[test]
    fn run_does_not_write_file_without_apply() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let manifest = make_manifest(vec![replace_rule("new.yml")]);
        let fetcher = MockFetcher::content(b"content\n".to_vec());
        let runner = MockPatchRunner::success(vec![]);
        let mut buf: Vec<u8> = Vec::new();

        // Act — do NOT call apply_outcomes
        run(&manifest, dir.path(), &fetcher, &runner, &mut buf).unwrap();

        // Assert — file must NOT exist
        assert!(
            !dir.path().join("new.yml").exists(),
            "preview must not write files"
        );
    }

    // ------------------------------------------------------------------
    // delete strategy
    // ------------------------------------------------------------------

    #[test]
    fn delete_removes_existing_file() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("old.yml"), b"content").unwrap();
        let manifest = make_manifest(vec![delete_rule("old.yml")]);
        let fetcher = MockFetcher::not_found();
        let runner = MockPatchRunner::success(vec![]);
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let (code, actions) = run(&manifest, dir.path(), &fetcher, &runner, &mut buf).unwrap();
        apply_outcomes(actions).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS: {out}");
        assert!(out.contains("[DELETED]"), "expected DELETED: {out}");
        assert!(
            !dir.path().join("old.yml").exists(),
            "file should be deleted"
        );
    }

    #[test]
    fn delete_skipped_when_file_absent() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let manifest = make_manifest(vec![delete_rule("nonexistent.yml")]);
        let fetcher = MockFetcher::not_found();
        let runner = MockPatchRunner::success(vec![]);
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let (code, _actions) = run(&manifest, dir.path(), &fetcher, &runner, &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS: {out}");
        assert!(out.contains("[SKIPPED]"), "expected SKIPPED: {out}");
    }

    // ------------------------------------------------------------------
    // create_only strategy
    // ------------------------------------------------------------------

    #[test]
    fn create_only_creates_when_absent() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let manifest = make_manifest(vec![create_only_rule("new.json")]);
        let fetcher = MockFetcher::content(b"{}".to_vec());
        let runner = MockPatchRunner::success(vec![]);
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let (code, actions) = run(&manifest, dir.path(), &fetcher, &runner, &mut buf).unwrap();
        apply_outcomes(actions).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS: {out}");
        assert!(out.contains("[CHANGED]"), "expected CHANGED: {out}");
        assert!(dir.path().join("new.json").exists());
    }

    #[test]
    fn create_only_skipped_when_exists() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("existing.json"), b"{}").unwrap();
        let manifest = make_manifest(vec![create_only_rule("existing.json")]);
        let fetcher = MockFetcher::content(b"new content".to_vec());
        let runner = MockPatchRunner::success(vec![]);
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let (code, _actions) = run(&manifest, dir.path(), &fetcher, &runner, &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS: {out}");
        assert!(out.contains("[SKIPPED]"), "expected SKIPPED: {out}");
    }

    // ------------------------------------------------------------------
    // patch strategy
    // ------------------------------------------------------------------

    #[test]
    fn patch_conflict_returns_failure() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let manifest = make_manifest(vec![Rule {
            path: String::from("Cargo.toml"),
            strategy: Strategy::Patch,
            source: None,
            patch: Some(String::from("test.patch")),
        }]);
        // Create the patch file so runner sees a valid path
        std::fs::write(dir.path().join("test.patch"), b"").unwrap();
        let fetcher = MockFetcher::content(b"upstream\n".to_vec());
        let runner = MockPatchRunner::conflict("hunk 1 failed");
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let (code, _actions) = run(&manifest, dir.path(), &fetcher, &runner, &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::FAILURE), "expected FAILURE: {out}");
        assert!(out.contains("[WARN"), "expected WARN tag: {out}");
    }

    // ------------------------------------------------------------------
    // apply_outcomes
    // ------------------------------------------------------------------

    #[test]
    fn apply_creates_parent_dirs() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let local_path = dir.path().join("a/b/c/file.txt");
        let actions = vec![ApplyAction::Write {
            local_path: local_path.clone(),
            content: b"hello".to_vec(),
        }];

        // Act
        apply_outcomes(actions).unwrap();

        // Assert
        assert_eq!(std::fs::read(&local_path).unwrap(), b"hello");
    }

    #[test]
    fn apply_removes_directory_for_deleted() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("subdir");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("file"), b"content").unwrap();
        let actions = vec![ApplyAction::Delete {
            local_path: sub.clone(),
        }];

        // Act
        apply_outcomes(actions).unwrap();

        // Assert
        assert!(!sub.exists(), "directory should be deleted");
    }

    fn _use_dir(_dir: &TempDir) {}
}
