//! `--patch-refresh` mode: regenerate patch files from upstream diff.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use anyhow::Context as _;
use gh_sync_manifest::{self as manifest, Manifest, Strategy};

use crate::diff::unified_diff;
use crate::output::{StatusTag, emit_status};
use crate::upstream::{FetchResult, UpstreamFetcher};

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

enum WriteOutcome {
    /// Patch file already had the correct content — not rewritten.
    Unchanged,
    /// Patch file was written (newly created or updated).
    Written,
}

/// Write `content` to `<repo_root>/<patch_path>`, creating parent directories
/// as needed.
///
/// Always writes, even when `content` is empty — an empty patch file is valid
/// and ensures that `sync` / `validate` do not fail with "not found".
///
/// Returns [`WriteOutcome::Unchanged`] when the file already contains the same
/// bytes so the caller can avoid emitting a spurious `CHANGED` status.
///
/// # Errors
///
/// Returns an error when directory creation or the file write fails.
fn write_patch(repo_root: &Path, patch_path: &str, content: &str) -> anyhow::Result<WriteOutcome> {
    let full = repo_root.join(patch_path);

    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create patch dir: {}", full.display()))?;
    }

    // No-op when the file already has the target content.
    if std::fs::read(&full).ok().as_deref() == Some(content.as_bytes()) {
        return Ok(WriteOutcome::Unchanged);
    }

    std::fs::write(&full, content.as_bytes())
        .with_context(|| format!("failed to write patch file: {}", full.display()))?;
    Ok(WriteOutcome::Written)
}

/// Regenerate `.patch` files for all `strategy: patch` rules.
///
/// For each patch rule:
/// 1. Fetch the upstream content.
/// 2. Read the local file (empty bytes when absent).
/// 3. Compute `diff -u upstream local`.
/// 4. Write the diff to the resolved patch file path.
///
/// Rules with other strategies are silently skipped.
///
/// Returns `ExitCode::FAILURE` when any rule encounters an error.
///
/// # Errors
/// Propagates I/O errors from `w`.
pub fn run(
    manifest: &Manifest,
    repo_root: &Path,
    fetcher: &dyn UpstreamFetcher,
    w: &mut dyn Write,
) -> std::io::Result<ExitCode> {
    let mut has_error = false;

    for rule in &manifest.files {
        if rule.strategy != Strategy::Patch {
            continue;
        }

        let local_path = repo_root.join(&rule.path);
        let local_bytes = std::fs::read(&local_path).unwrap_or_default();

        let upstream =
            match fetcher.fetch(&manifest.upstream.repo, &manifest.upstream.ref_, &rule.path) {
                Err(e) => {
                    emit_status(
                        w,
                        StatusTag::Fail,
                        &rule.path,
                        rule.strategy,
                        Some(&format!("fetch error: {e}")),
                    )?;
                    has_error = true;
                    continue;
                }
                Ok(FetchResult::NotFound) => {
                    emit_status(
                        w,
                        StatusTag::Fail,
                        &rule.path,
                        rule.strategy,
                        Some(&format!("upstream path not found: {}", rule.path)),
                    )?;
                    has_error = true;
                    continue;
                }
                Ok(FetchResult::Content(bytes)) => bytes,
            };

        let diff = match unified_diff(&rule.path, &upstream, &local_bytes) {
            Ok(d) => d,
            Err(e) => {
                emit_status(
                    w,
                    StatusTag::Fail,
                    &rule.path,
                    rule.strategy,
                    Some(&format!("diff error: {e}")),
                )?;
                has_error = true;
                continue;
            }
        };

        let patch_path = manifest::resolve_patch_path(rule);
        match write_patch(repo_root, &patch_path, &diff) {
            Ok(WriteOutcome::Unchanged) => {
                emit_status(
                    w,
                    StatusTag::Ok,
                    &rule.path,
                    rule.strategy,
                    Some("no changes"),
                )?;
            }
            Ok(WriteOutcome::Written) => {
                let detail = if diff.is_empty() {
                    format!("wrote {patch_path} (empty: local matches upstream)")
                } else {
                    format!("wrote {patch_path}")
                };
                emit_status(
                    w,
                    StatusTag::Changed,
                    &rule.path,
                    rule.strategy,
                    Some(&detail),
                )?;
            }
            Err(e) => {
                emit_status(
                    w,
                    StatusTag::Fail,
                    &rule.path,
                    rule.strategy,
                    Some(&e.to_string()),
                )?;
                has_error = true;
            }
        }
    }

    Ok(if has_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
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

    use super::*;
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

    fn patch_rule(path: &str) -> Rule {
        Rule {
            path: path.to_owned(),
            strategy: Strategy::Patch,
            source: None,
            patch: None,
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

    // ------------------------------------------------------------------
    // Non-patch rules are skipped silently
    // ------------------------------------------------------------------

    #[test]
    fn non_patch_rules_are_skipped() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let manifest = make_manifest(vec![replace_rule("ci.yml")]);
        let fetcher = MockFetcher::content(b"upstream".to_vec());
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let code = run(&manifest, dir.path(), &fetcher, &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS: {out}");
        // No status lines written for non-patch rules
        assert!(
            out.is_empty(),
            "expected no output for non-patch rules: {out}"
        );
    }

    // ------------------------------------------------------------------
    // Patch with diff writes patch file
    // ------------------------------------------------------------------

    #[cfg_attr(miri, ignore)]
    #[test]
    fn patch_with_diff_writes_patch_file() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), b"local content\n").unwrap();
        let manifest = make_manifest(vec![patch_rule("Cargo.toml")]);
        let fetcher = MockFetcher::content(b"upstream content\n".to_vec());
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let code = run(&manifest, dir.path(), &fetcher, &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS: {out}");
        assert!(out.contains("[CHANGED]"), "expected CHANGED: {out}");

        let patch_path = dir.path().join(".github/gh-sync/patches/Cargo.toml.patch");
        assert!(patch_path.exists(), "patch file should be written");
        let patch_content = String::from_utf8(std::fs::read(&patch_path).unwrap()).unwrap();
        assert!(
            patch_content.contains("-upstream content"),
            "patch should show upstream removal"
        );
        assert!(
            patch_content.contains("+local content"),
            "patch should show local addition"
        );
    }

    // ------------------------------------------------------------------
    // Identical content: empty patch file is created (so validate passes)
    // ------------------------------------------------------------------

    #[cfg_attr(miri, ignore)]
    #[test]
    fn identical_content_creates_empty_patch_file() {
        // Arrange — local matches upstream exactly; no patch file yet
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), b"same\n").unwrap();
        let manifest = make_manifest(vec![patch_rule("Cargo.toml")]);
        let fetcher = MockFetcher::content(b"same\n".to_vec());
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let code = run(&manifest, dir.path(), &fetcher, &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS: {out}");
        // Reports CHANGED because the empty file was newly created
        assert!(out.contains("[CHANGED]"), "expected CHANGED: {out}");
        let patch_path = dir.path().join(".github/gh-sync/patches/Cargo.toml.patch");
        assert!(patch_path.exists(), "empty patch file should be created");
        assert_eq!(
            std::fs::read(&patch_path).unwrap(),
            b"",
            "patch file content should be empty"
        );
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn identical_content_already_empty_patch_reports_no_changes() {
        // Arrange — patch file already exists and is empty
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), b"same\n").unwrap();
        let patch_dir = dir.path().join(".github/gh-sync/patches");
        std::fs::create_dir_all(&patch_dir).unwrap();
        std::fs::write(patch_dir.join("Cargo.toml.patch"), b"").unwrap();
        let manifest = make_manifest(vec![patch_rule("Cargo.toml")]);
        let fetcher = MockFetcher::content(b"same\n".to_vec());
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let code = run(&manifest, dir.path(), &fetcher, &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS: {out}");
        assert!(
            out.contains("[OK"),
            "expected OK (already up-to-date): {out}"
        );
    }

    // ------------------------------------------------------------------
    // Local file absent: treated as empty
    // ------------------------------------------------------------------

    #[cfg_attr(miri, ignore)]
    #[test]
    fn absent_local_file_treated_as_empty() {
        // Arrange — no local Cargo.toml
        let dir = tempfile::tempdir().unwrap();
        let manifest = make_manifest(vec![patch_rule("Cargo.toml")]);
        let fetcher = MockFetcher::content(b"upstream content\n".to_vec());
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let code = run(&manifest, dir.path(), &fetcher, &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS: {out}");
        let patch_path = dir.path().join(".github/gh-sync/patches/Cargo.toml.patch");
        assert!(patch_path.exists(), "patch file should be written: {out}");
    }

    // ------------------------------------------------------------------
    // Upstream fetch error → FAIL
    // ------------------------------------------------------------------

    #[test]
    fn fetch_error_returns_failure() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let manifest = make_manifest(vec![patch_rule("Cargo.toml")]);
        let fetcher = MockFetcher::error("network error");
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let code = run(&manifest, dir.path(), &fetcher, &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::FAILURE), "expected FAILURE: {out}");
        assert!(out.contains("[FAIL"), "expected FAIL: {out}");
    }

    // ------------------------------------------------------------------
    // Upstream not found → FAIL
    // ------------------------------------------------------------------

    #[test]
    fn upstream_not_found_returns_failure() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let manifest = make_manifest(vec![patch_rule("Cargo.toml")]);
        let fetcher = MockFetcher::not_found();
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let code = run(&manifest, dir.path(), &fetcher, &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::FAILURE), "expected FAILURE: {out}");
        assert!(out.contains("[FAIL"), "expected FAIL: {out}");
    }
}
