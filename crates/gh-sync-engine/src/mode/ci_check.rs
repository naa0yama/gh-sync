//! `--ci-check` mode: drift detection with optional GHA annotations.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use gh_sync_manifest::{self as manifest, Manifest, Rule, Strategy};

use crate::diff::unified_diff;
use crate::output::{
    DriftOutcome, StatusTag, Summary, build_diff_context_header, build_pr_comment, emit_diff,
    emit_drift_summary, emit_gha_annotations, emit_status,
};
use crate::strategy::patch::PatchRunner;
use crate::strategy::{self, StrategyResult};
use crate::upstream::{FetchResult, UpstreamFetcher};

/// Run drift-detection mode.
///
/// Each rule is evaluated without writing any files.  When the local state
/// diverges from the expected state, the rule is marked as drifted.
///
/// If `GITHUB_ACTIONS=true`, GitHub Actions annotations are written to `w`.
/// If the run is also associated with a PR (`GITHUB_REF` matches
/// `refs/pull/*/merge`), a comment body is posted via `gh pr comment`.
///
/// Returns `ExitCode::FAILURE` when any drift, conflict, or error is found.
///
/// # Errors
/// Propagates I/O errors from `w`.
pub fn run(
    manifest: &Manifest,
    repo_root: &Path,
    fetcher: &dyn UpstreamFetcher,
    patch_runner: &dyn PatchRunner,
    w: &mut dyn Write,
) -> std::io::Result<ExitCode> {
    let mut drift_outcomes: Vec<DriftOutcome<'_>> = Vec::new();
    let mut has_error = false;

    for rule in &manifest.files {
        if rule.strategy == Strategy::Ignore {
            continue;
        }

        let local_path = repo_root.join(&rule.path);
        let local_bytes: Option<Vec<u8>> = std::fs::read(&local_path).ok();

        let (drifted, detail, diff, error) = evaluate_drift(
            rule,
            manifest,
            repo_root,
            local_bytes.as_deref(),
            fetcher,
            patch_runner,
        );

        if error {
            has_error = true;
        }

        let tag = if error {
            StatusTag::Fail
        } else if drifted {
            StatusTag::Drift
        } else {
            StatusTag::Ok
        };
        emit_status(w, tag, &rule.path, rule.strategy, Some(&detail))?;
        if drifted && !diff.is_empty() {
            emit_diff(w, &diff)?;
        }

        drift_outcomes.push(DriftOutcome {
            rule,
            drifted,
            detail,
            diff,
        });
    }

    // Summary footer
    let ds = Summary::from_drift_outcomes(&drift_outcomes);
    emit_drift_summary(w, &ds)?;

    // GHA annotations (when running in GitHub Actions)
    if std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true") {
        emit_gha_annotations(w, &drift_outcomes)?;
        maybe_post_pr_comment(&drift_outcomes);
    }

    let any_drift = ds.drifted > 0;
    Ok(if any_drift || has_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

/// Evaluate whether `rule` is drifted.
///
/// Returns `(drifted, detail, diff, is_error)`.
fn evaluate_drift(
    rule: &Rule,
    manifest: &Manifest,
    repo_root: &Path,
    local_bytes: Option<&[u8]>,
    fetcher: &dyn UpstreamFetcher,
    patch_runner: &dyn PatchRunner,
) -> (bool, String, String, bool) {
    match rule.strategy {
        // `Ignore` rules are skipped by the caller before `evaluate_drift` is
        // invoked; this arm keeps the match exhaustive.
        Strategy::Ignore => (false, String::from("ignored"), String::new(), false),
        Strategy::Delete => {
            if local_bytes.is_some() {
                (
                    true,
                    String::from("file exists but should be deleted"),
                    String::new(),
                    false,
                )
            } else {
                (false, String::from("already absent"), String::new(), false)
            }
        }
        Strategy::CreateOnly => {
            // Only check existence — content match is irrelevant for create_only
            if local_bytes.is_none() {
                (true, String::from("file not found"), String::new(), false)
            } else {
                (false, String::from("file exists"), String::new(), false)
            }
        }
        Strategy::Replace | Strategy::Patch => {
            let source = if rule.strategy == Strategy::Replace {
                rule.source.as_deref().unwrap_or(&rule.path)
            } else {
                &rule.path
            };

            let upstream =
                match fetcher.fetch(&manifest.upstream.repo, &manifest.upstream.ref_, source) {
                    Err(e) => {
                        return (false, format!("fetch error: {e}"), String::new(), true);
                    }
                    Ok(FetchResult::NotFound) => {
                        return (
                            false,
                            format!("upstream not found: {source}"),
                            String::new(),
                            false,
                        );
                    }
                    Ok(FetchResult::Content(bytes)) => bytes,
                };

            let expected = if rule.strategy == Strategy::Replace {
                upstream
            } else {
                // Apply patch to get expected content
                let patch_path = manifest::resolve_patch_path(rule);
                let full_patch = repo_root.join(&patch_path);
                match strategy::patch::apply(&upstream, local_bytes, &full_patch, patch_runner) {
                    StrategyResult::Changed { content } => content,
                    StrategyResult::Conflict { message } => {
                        return (false, format!("conflict: {message}"), String::new(), true);
                    }
                    StrategyResult::Error(msg) => {
                        return (false, format!("patch error: {msg}"), String::new(), true);
                    }
                    // Unchanged, Skipped, Deleted — fall back to raw upstream
                    _ => upstream,
                }
            };

            let local_content = local_bytes.unwrap_or(b"");
            if local_content == expected.as_slice() {
                (false, String::from("up to date"), String::new(), false)
            } else {
                let body = unified_diff(&rule.path, local_content, &expected).unwrap_or_default();
                let diff = if body.is_empty() {
                    String::new()
                } else {
                    let header =
                        build_diff_context_header(&manifest.upstream.repo, &manifest.upstream.ref_);
                    format!("{header}\n{body}")
                };
                (true, String::from("upstream has changes"), diff, false)
            }
        }
    }
}

/// Post a PR comment when running in a GHA PR context.
///
/// # NOTEST(io): posting a PR comment requires a live `gh` CLI and GitHub token.
///
/// TODO: extract `gh pr comment` call to the `gh-sync` binary crate to keep
/// this engine crate free of process-spawning I/O.
fn maybe_post_pr_comment(outcomes: &[DriftOutcome<'_>]) {
    let Ok(ref_) = std::env::var("GITHUB_REF") else {
        return;
    };
    // Extract PR number from refs/pull/<number>/merge
    let Some(pr_number) = extract_pr_number(&ref_) else {
        return;
    };

    let Some(body) = build_pr_comment(outcomes) else {
        return;
    };

    let _ = std::process::Command::new("gh")
        .args(["pr", "comment", &pr_number.to_string(), "--body", &body])
        .status();
}

fn extract_pr_number(ref_: &str) -> Option<u64> {
    // refs/pull/<number>/merge
    let rest = ref_.strip_prefix("refs/pull/")?;
    let number_str = rest.strip_suffix("/merge")?;
    number_str.parse().ok()
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
    // replace
    // ------------------------------------------------------------------

    #[test]
    fn replace_up_to_date_returns_success() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ci.yml"), b"upstream\n").unwrap();
        let manifest = make_manifest(vec![replace_rule("ci.yml")]);
        let fetcher = MockFetcher::content(b"upstream\n".to_vec());
        let runner = MockPatchRunner::success(vec![]);
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let code = run(&manifest, dir.path(), &fetcher, &runner, &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS: {out}");
        assert!(out.contains("[OK"), "expected OK: {out}");
    }

    #[cfg_attr(
        miri,
        ignore = "spawns gh process via maybe_post_pr_comment when GITHUB_ACTIONS is set"
    )]
    #[test]
    fn replace_drift_returns_failure() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ci.yml"), b"local\n").unwrap();
        let manifest = make_manifest(vec![replace_rule("ci.yml")]);
        let fetcher = MockFetcher::content(b"upstream\n".to_vec());
        let runner = MockPatchRunner::success(vec![]);
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let code = run(&manifest, dir.path(), &fetcher, &runner, &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::FAILURE), "expected FAILURE: {out}");
        assert!(out.contains("[DRIFT"), "expected DRIFT: {out}");
        assert!(
            out.contains("# a/ = local, b/ = upstream (owner/repo@main)"),
            "missing diff context header: {out}"
        );
    }

    #[test]
    fn fetch_error_returns_failure() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let manifest = make_manifest(vec![replace_rule("ci.yml")]);
        let fetcher = MockFetcher::error("network error");
        let runner = MockPatchRunner::success(vec![]);
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let code = run(&manifest, dir.path(), &fetcher, &runner, &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::FAILURE), "expected FAILURE: {out}");
        assert!(out.contains("[FAIL"), "expected FAIL: {out}");
    }

    // ------------------------------------------------------------------
    // delete
    // ------------------------------------------------------------------

    #[cfg_attr(
        miri,
        ignore = "spawns gh process via maybe_post_pr_comment when GITHUB_ACTIONS is set"
    )]
    #[test]
    fn delete_drift_when_file_exists() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("old.yml"), b"content").unwrap();
        let manifest = make_manifest(vec![delete_rule("old.yml")]);
        let fetcher = MockFetcher::not_found();
        let runner = MockPatchRunner::success(vec![]);
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let code = run(&manifest, dir.path(), &fetcher, &runner, &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::FAILURE), "expected FAILURE: {out}");
        assert!(out.contains("[DRIFT"), "expected DRIFT: {out}");
        // File must NOT be deleted in ci-check mode
        assert!(
            dir.path().join("old.yml").exists(),
            "ci-check must not delete files"
        );
    }

    #[test]
    fn delete_ok_when_file_absent() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let manifest = make_manifest(vec![delete_rule("gone.yml")]);
        let fetcher = MockFetcher::not_found();
        let runner = MockPatchRunner::success(vec![]);
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let code = run(&manifest, dir.path(), &fetcher, &runner, &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS: {out}");
    }

    // ------------------------------------------------------------------
    // create_only
    // ------------------------------------------------------------------

    #[cfg_attr(miri, ignore = "tempfile I/O not supported under Miri")]
    #[test]
    fn create_only_drift_when_file_missing() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let manifest = make_manifest(vec![create_only_rule("config.json")]);
        let fetcher = MockFetcher::not_found();
        let runner = MockPatchRunner::success(vec![]);
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let code = run(&manifest, dir.path(), &fetcher, &runner, &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::FAILURE), "expected FAILURE: {out}");
    }

    #[test]
    fn create_only_ok_when_file_exists_regardless_of_content() {
        // Arrange — local content differs from "upstream" but create_only ignores content
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), b"local content").unwrap();
        let manifest = make_manifest(vec![create_only_rule("config.json")]);
        let fetcher = MockFetcher::content(b"upstream content".to_vec());
        let runner = MockPatchRunner::success(vec![]);
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let code = run(&manifest, dir.path(), &fetcher, &runner, &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS: {out}");
        assert!(out.contains("[OK"), "expected OK: {out}");
    }

    // ------------------------------------------------------------------
    // extract_pr_number
    // ------------------------------------------------------------------

    #[test]
    fn extract_pr_number_valid() {
        assert_eq!(extract_pr_number("refs/pull/123/merge"), Some(123));
        assert_eq!(extract_pr_number("refs/pull/1/merge"), Some(1));
    }

    #[test]
    fn extract_pr_number_invalid() {
        assert_eq!(extract_pr_number("refs/heads/main"), None);
        assert_eq!(extract_pr_number("refs/pull/abc/merge"), None);
        assert_eq!(extract_pr_number("refs/pull/123/head"), None);
    }

    // ------------------------------------------------------------------
    // DriftSummary counts
    // ------------------------------------------------------------------

    #[test]
    fn drift_summary_counts_drifted_and_clean() {
        // Arrange
        let r = replace_rule("x");
        let outcomes = vec![
            DriftOutcome {
                rule: &r,
                drifted: true,
                detail: String::new(),
                diff: String::new(),
            },
            DriftOutcome {
                rule: &r,
                drifted: false,
                detail: String::new(),
                diff: String::new(),
            },
            DriftOutcome {
                rule: &r,
                drifted: true,
                detail: String::new(),
                diff: String::new(),
            },
        ];

        // Act
        let s = Summary::from_drift_outcomes(&outcomes);

        // Assert
        assert_eq!(s.drifted, 2);
        assert_eq!(s.up_to_date, 1);
    }
}
