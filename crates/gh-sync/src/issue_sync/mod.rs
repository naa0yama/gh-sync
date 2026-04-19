/// CLI argument definitions for the `issue-sync` subcommand.
pub mod cli;

use std::io::{self, Write as _};
use std::path::Path;
use std::process::ExitCode;

use anyhow::Context as _;
use cli::IssueSyncArgs;
use gh_sync_engine::mode::ci_check;
use gh_sync_engine::output::build_pr_comment;

use crate::sync::manifest;
use crate::sync::repo::{GhRepoClientImpl, RepoCiCheckReport, ci_check_structured};
use crate::sync::runner::{GhRunner, SystemGhRunner};
use crate::sync::strategy::patch::RealPatchRunner;
use crate::sync::upstream::GhFetcher;
use crate::sync::upstream_manifest;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Execute the `issue-sync` subcommand.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn execute(args: &IssueSyncArgs) -> ExitCode {
    let runner = SystemGhRunner;
    match run(args, &runner) {
        Ok(has_drift) => {
            if has_drift {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            tracing::error!("issue-sync failed: {e:#}");
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------------------
// Core logic
// ---------------------------------------------------------------------------

/// Core logic for `issue-sync`, parameterised over the runner for testability.
///
/// Returns `true` when drift was detected (and an issue was upserted).
/// Returns `false` when no drift (and any existing issue was closed).
///
/// # Errors
///
/// Returns an error when the manifest cannot be resolved, a drift check fails
/// irrecoverably, or any `gh` CLI call fails.
pub fn run(args: &IssueSyncArgs, runner: &dyn GhRunner) -> anyhow::Result<bool> {
    let repo_root = Path::new(".");
    let fetcher = GhFetcher;
    let patch_runner = RealPatchRunner;

    // Resolve the effective manifest.
    let manifest =
        upstream_manifest::resolve(args.upstream_manifest.as_deref(), &args.manifest, &fetcher)
            .with_context(|| format!("failed to resolve manifest '{}'", args.manifest.display()))?;

    if let Err(e) = manifest::validate_schema(&manifest) {
        anyhow::bail!("manifest validation failed: {e}");
    }

    // File drift check.
    let mut w = io::stderr();
    let file_report =
        ci_check::run_structured(&manifest, repo_root, &fetcher, &patch_runner, &mut w)
            .context("file drift check failed")?;

    // Repo settings drift check (only when manifest has spec:).
    let repo_report: Option<RepoCiCheckReport> = manifest.spec.as_ref().and_then(|spec| {
        let client = GhRepoClientImpl::new();
        match ci_check_structured(spec, &client) {
            Ok(r) => Some(r),
            Err(e) => {
                tracing::warn!("repo drift check failed (skipped): {e:#}");
                None
            }
        }
    });

    let has_drift =
        file_report.has_any_drift() || repo_report.as_ref().is_some_and(|r| r.has_actions);

    // Resolve target repo name.
    let repo = resolve_repo(args.repo.as_deref(), runner)?;

    // Build issue body when there is drift.
    let issue_body = if has_drift {
        Some(build_issue_body(&file_report, repo_report.as_ref()))
    } else {
        None
    };

    manage_drift_issue(
        has_drift,
        issue_body.as_deref(),
        &repo,
        &args.label,
        &args.title,
        runner,
    )?;

    Ok(has_drift)
}

// ---------------------------------------------------------------------------
// Issue body builder
// ---------------------------------------------------------------------------

fn build_issue_body(
    file_report: &ci_check::CiCheckReport<'_>,
    repo_report: Option<&RepoCiCheckReport>,
) -> String {
    use std::fmt::Write as _;

    let mut body = String::new();

    // File drift section.
    let _ = writeln!(body, "## File drift");
    if let Some(comment) = build_pr_comment(&file_report.drift_outcomes) {
        // Strip the leading "## gh-sync drift detected\n\n" header since we have our own.
        let stripped = comment
            .strip_prefix("## gh-sync drift detected\n\n")
            .unwrap_or(&comment);
        body.push_str(stripped);
    } else {
        let _ = writeln!(body, "No file drift detected.");
    }

    // Repo settings section (when present and drifted).
    if let Some(report) = repo_report {
        let _ = writeln!(body, "\n## Repo settings drift");
        if report.has_actions {
            let _ = writeln!(body, "```");
            body.push_str(&report.preview_text);
            let _ = writeln!(body, "```");
        } else {
            let _ = writeln!(body, "No repo settings drift detected.");
        }
    }

    body.push_str("\nRun `gh-sync sync` locally to apply upstream changes.\n");
    body
}

// ---------------------------------------------------------------------------
// Issue lifecycle
// ---------------------------------------------------------------------------

/// Upsert or close the drift tracking issue.
///
/// - Drift detected: create issue if absent, edit body if present.
/// - No drift: close issue with a comment if present, do nothing if absent.
///
/// # Errors
///
/// Returns an error when any `gh` CLI call fails.
pub fn manage_drift_issue(
    has_drift: bool,
    issue_body: Option<&str>,
    repo: &str,
    label: &str,
    title: &str,
    runner: &dyn GhRunner,
) -> anyhow::Result<()> {
    let existing = find_open_issue(repo, label, runner)?;

    if has_drift {
        let body = issue_body.unwrap_or("");
        if let Some(number) = existing {
            edit_issue(runner, repo, number, body)?;
        } else {
            ensure_label(runner, repo, label)?;
            create_issue(runner, repo, label, title, body)?;
        }
    } else if let Some(number) = existing {
        // No drift — close any open tracking issue.
        close_issue(runner, repo, number)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Repo name resolution
// ---------------------------------------------------------------------------

fn resolve_repo(arg_repo: Option<&str>, runner: &dyn GhRunner) -> anyhow::Result<String> {
    if let Some(r) = arg_repo {
        return Ok(r.to_owned());
    }

    // Try GITHUB_REPOSITORY env var first (available in GHA).
    if let Ok(env_repo) = std::env::var("GITHUB_REPOSITORY")
        && !env_repo.is_empty()
    {
        return Ok(env_repo);
    }

    let out = runner
        .run(
            &[
                "repo",
                "view",
                "--json",
                "nameWithOwner",
                "--jq",
                ".nameWithOwner",
            ],
            None,
        )
        .context("failed to spawn `gh`")?;

    if !out.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!(
            "failed to detect repository via `gh repo view`: {stderr}\n\
             Use --repo owner/repo to specify it explicitly."
        );
    }

    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

// ---------------------------------------------------------------------------
// gh CLI helpers
// ---------------------------------------------------------------------------

/// Find the lowest-numbered open issue with `label` in `repo`.
///
/// Returns `None` when no matching open issue exists.
fn find_open_issue(repo: &str, label: &str, runner: &dyn GhRunner) -> anyhow::Result<Option<u64>> {
    let out = runner
        .run(
            &[
                "issue", "list", "--repo", repo, "--label", label, "--state", "open", "--json",
                "number", "--limit", "10",
            ],
            None,
        )
        .context("failed to spawn `gh`")?;

    if !out.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("gh issue list failed: {stderr}");
    }

    let json: serde_json::Value =
        serde_json::from_slice(&out.stdout).context("failed to parse gh issue list JSON")?;

    Ok(json
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|v| v.get("number")?.as_u64())
        .min())
}

/// Edit the body of an existing issue.
fn edit_issue(runner: &dyn GhRunner, repo: &str, number: u64, body: &str) -> anyhow::Result<()> {
    let out = runner
        .run(
            &[
                "issue",
                "edit",
                &number.to_string(),
                "--repo",
                repo,
                "--body-file",
                "-",
            ],
            Some(body.as_bytes()),
        )
        .context("failed to spawn `gh`")?;

    if !out.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("gh issue edit failed: {stderr}");
    }
    Ok(())
}

/// Ensure a label exists in `repo` (idempotent via `--force`).
fn ensure_label(runner: &dyn GhRunner, repo: &str, label: &str) -> anyhow::Result<()> {
    let out = runner
        .run(
            &[
                "label",
                "create",
                label,
                "--repo",
                repo,
                "--force",
                "--description",
                "Upstream drift detected by gh-sync",
            ],
            None,
        )
        .context("failed to spawn `gh`")?;

    if !out.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("gh label create failed: {stderr}");
    }
    Ok(())
}

/// Create a new drift tracking issue.
fn create_issue(
    runner: &dyn GhRunner,
    repo: &str,
    label: &str,
    title: &str,
    body: &str,
) -> anyhow::Result<()> {
    let out = runner
        .run(
            &[
                "issue",
                "create",
                "--repo",
                repo,
                "--label",
                label,
                "--title",
                title,
                "--body-file",
                "-",
            ],
            Some(body.as_bytes()),
        )
        .context("failed to spawn `gh`")?;

    if !out.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("gh issue create failed: {stderr}");
    }

    let url = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    let mut stdout = io::stdout();
    let _ = writeln!(stdout, "[OK] drift issue: {url}");
    Ok(())
}

/// Post a closing comment and close the issue.
fn close_issue(runner: &dyn GhRunner, repo: &str, number: u64) -> anyhow::Result<()> {
    // Comment first.
    let sha = std::env::var("GITHUB_SHA").unwrap_or_else(|_| String::from("(unknown)"));
    let comment_body = format!("Drift resolved at {sha}.");
    let out = runner
        .run(
            &[
                "issue",
                "comment",
                &number.to_string(),
                "--repo",
                repo,
                "--body",
                &comment_body,
            ],
            None,
        )
        .context("failed to spawn `gh`")?;

    if !out.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("gh issue comment failed: {stderr}");
    }

    // Then close.
    let out = runner
        .run(
            &["issue", "close", &number.to_string(), "--repo", repo],
            None,
        )
        .context("failed to spawn `gh`")?;

    if !out.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("gh issue close failed: {stderr}");
    }

    let mut stdout = io::stdout();
    let _ = writeln!(stdout, "[OK] drift issue #{number} closed");
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::indexing_slicing)]
    #![allow(clippy::panic)]

    use crate::sync::runner::{GhOutput, GhRunner};

    use super::*;

    // ---------------------------------------------------------------------------
    // Test helpers
    // ---------------------------------------------------------------------------

    struct MockRunner {
        calls: std::sync::Mutex<Vec<Vec<String>>>,
        responses: std::sync::Mutex<std::collections::VecDeque<GhOutput>>,
    }

    impl MockRunner {
        fn new(responses: Vec<GhOutput>) -> Self {
            Self {
                calls: std::sync::Mutex::new(Vec::new()),
                responses: std::sync::Mutex::new(responses.into()),
            }
        }

        fn ok(stdout: &str) -> GhOutput {
            GhOutput {
                exit_code: Some(0),
                stdout: stdout.as_bytes().to_vec(),
                stderr: vec![],
            }
        }

        fn fail(stderr: &str) -> GhOutput {
            GhOutput {
                exit_code: Some(1),
                stdout: vec![],
                stderr: stderr.as_bytes().to_vec(),
            }
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl GhRunner for MockRunner {
        fn run(&self, args: &[&str], _stdin: Option<&[u8]>) -> anyhow::Result<GhOutput> {
            self.calls
                .lock()
                .unwrap()
                .push(args.iter().map(|s| String::from(*s)).collect());
            let response = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Self::ok(""));
            Ok(response)
        }
    }

    // Helper to build an empty issue list JSON response.
    fn no_issues() -> GhOutput {
        MockRunner::ok("[]")
    }

    // Helper to build a single-issue JSON response.
    fn one_issue(number: u64) -> GhOutput {
        MockRunner::ok(&format!(r#"[{{"number":{number}}}]"#))
    }

    // ---------------------------------------------------------------------------
    // manage_drift_issue — no drift, no existing issue
    // ---------------------------------------------------------------------------

    #[test]
    fn no_drift_no_issue_does_nothing() {
        let runner = MockRunner::new(vec![no_issues()]);
        manage_drift_issue(false, None, "owner/repo", "gh-sync-drift", "title", &runner).unwrap();
        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains(&String::from("list")));
    }

    // ---------------------------------------------------------------------------
    // manage_drift_issue — no drift, existing issue → close
    // ---------------------------------------------------------------------------

    #[test]
    fn no_drift_with_open_issue_closes_it() {
        let runner = MockRunner::new(vec![
            one_issue(42),      // issue list
            MockRunner::ok(""), // issue comment
            MockRunner::ok(""), // issue close
        ]);
        manage_drift_issue(false, None, "owner/repo", "gh-sync-drift", "title", &runner).unwrap();
        let calls = runner.calls();
        assert_eq!(calls.len(), 3);
        assert!(calls[1].contains(&String::from("comment")));
        assert!(calls[2].contains(&String::from("close")));
    }

    // ---------------------------------------------------------------------------
    // manage_drift_issue — drift, no existing issue → create
    // ---------------------------------------------------------------------------

    #[test]
    fn drift_with_no_issue_creates_issue() {
        let runner = MockRunner::new(vec![
            no_issues(),                                              // issue list
            MockRunner::ok(""),                                       // label create
            MockRunner::ok("https://github.com/owner/repo/issues/1"), // issue create
        ]);
        manage_drift_issue(
            true,
            Some("drift body"),
            "owner/repo",
            "gh-sync-drift",
            "title",
            &runner,
        )
        .unwrap();
        let calls = runner.calls();
        assert_eq!(calls.len(), 3);
        assert!(
            calls[1].contains(&String::from("create")),
            "label create missing: {calls:?}"
        );
        assert!(
            calls[2].contains(&String::from("create")),
            "issue create missing: {calls:?}"
        );
    }

    // ---------------------------------------------------------------------------
    // manage_drift_issue — drift, existing issue → edit
    // ---------------------------------------------------------------------------

    #[test]
    fn drift_with_open_issue_edits_it() {
        let runner = MockRunner::new(vec![
            one_issue(7),       // issue list
            MockRunner::ok(""), // issue edit
        ]);
        manage_drift_issue(
            true,
            Some("new body"),
            "owner/repo",
            "gh-sync-drift",
            "title",
            &runner,
        )
        .unwrap();
        let calls = runner.calls();
        assert_eq!(calls.len(), 2);
        assert!(calls[1].contains(&String::from("edit")));
    }

    // ---------------------------------------------------------------------------
    // manage_drift_issue — gh issue list fails → error
    // ---------------------------------------------------------------------------

    #[test]
    fn issue_list_failure_returns_error() {
        let runner = MockRunner::new(vec![MockRunner::fail("internal server error")]);
        let result =
            manage_drift_issue(false, None, "owner/repo", "gh-sync-drift", "title", &runner);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("gh issue list") || msg.contains("internal server error"),
            "unexpected error: {msg}"
        );
    }

    // ---------------------------------------------------------------------------
    // find_open_issue — returns lowest number when multiple issues
    // ---------------------------------------------------------------------------

    #[test]
    fn find_open_issue_returns_lowest_number() {
        let json = r#"[{"number":10},{"number":3},{"number":7}]"#;
        let runner = MockRunner::new(vec![MockRunner::ok(json)]);
        let result = find_open_issue("owner/repo", "gh-sync-drift", &runner).unwrap();
        assert_eq!(result, Some(3));
    }

    #[test]
    fn find_open_issue_returns_none_for_empty_list() {
        let runner = MockRunner::new(vec![no_issues()]);
        let result = find_open_issue("owner/repo", "gh-sync-drift", &runner).unwrap();
        assert_eq!(result, None);
    }

    // ---------------------------------------------------------------------------
    // resolve_repo — explicit --repo arg
    // ---------------------------------------------------------------------------

    #[test]
    fn resolve_repo_uses_arg_when_provided() {
        struct NeverRunner;
        impl GhRunner for NeverRunner {
            fn run(&self, _: &[&str], _: Option<&[u8]>) -> anyhow::Result<GhOutput> {
                panic!("should not be called")
            }
        }
        // Clear GITHUB_REPOSITORY so arg path is tested.
        let result = resolve_repo(Some("my-owner/my-repo"), &NeverRunner);
        assert_eq!(result.unwrap(), "my-owner/my-repo");
    }

    // ---------------------------------------------------------------------------
    // resolve_repo — runner path
    // ---------------------------------------------------------------------------

    #[test]
    fn resolve_repo_calls_gh_when_arg_absent() {
        // Ensure GITHUB_REPOSITORY env is absent for this test.
        // SAFETY: single-threaded test, no concurrent env reads.
        unsafe { std::env::remove_var("GITHUB_REPOSITORY") };
        let runner = MockRunner::new(vec![MockRunner::ok("detected/repo\n")]);
        let result = resolve_repo(None, &runner).unwrap();
        assert_eq!(result, "detected/repo");
    }

    // ---------------------------------------------------------------------------
    // build_issue_body
    // ---------------------------------------------------------------------------

    #[test]
    fn build_issue_body_contains_both_sections() {
        use gh_sync_engine::output::{DriftOutcome, DriftSummary};
        use gh_sync_manifest::{Rule, Strategy};

        let rule = Rule {
            path: String::from("foo.txt"),
            strategy: Strategy::Replace,
            source: None,
            patch: None,
            preserve_markers: None,
        };
        let drift_outcomes = vec![DriftOutcome {
            rule: &rule,
            drifted: true,
            detail: String::from("upstream has changes"),
            diff: String::from("diff content"),
        }];
        let file_report = ci_check::CiCheckReport {
            drift_outcomes,
            summary: DriftSummary {
                drifted: 1,
                up_to_date: 0,
            },
            has_error: false,
        };
        let repo_report = RepoCiCheckReport {
            has_actions: true,
            preview_text: String::from("=== repo: owner/repo ===\n---\n1 changed\n"),
        };

        let body = build_issue_body(&file_report, Some(&repo_report));
        assert!(body.contains("## File drift"), "file drift section missing");
        assert!(
            body.contains("## Repo settings drift"),
            "repo drift section missing"
        );
        assert!(body.contains("foo.txt"), "file path missing");
        assert!(body.contains("owner/repo"), "repo name missing");
    }
}
