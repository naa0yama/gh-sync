//! Auto-detection of the fork / template parent for the current repository.
//!
//! Queries `gh api repos/{owner}/{repo}` and inspects the `parent` (fork) and
//! `template_repository` fields to determine if the current repo was derived
//! from an upstream template.  The result is used by `sync file`, `sync repo`,
//! and `init` to offer the upstream as a default `--upstream-manifest` value.

use anyhow::Context as _;

use crate::sync::runner::{GhRunner, run_checked};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Whether the detected upstream relationship is a fork or a template clone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParentSource {
    Fork,
    Template,
}

impl ParentSource {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Fork => "fork",
            Self::Template => "template",
        }
    }
}

/// An upstream repository that is the fork or template parent of the current repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamParent {
    /// `owner/name` format, e.g. `"naa0yama/boilerplate-rust"`.
    pub full_name: String,
    /// Default branch of the parent, e.g. `"main"`.
    pub default_branch: String,
    pub source: ParentSource,
}

/// Decision made by [`decide_upstream_manifest`].
#[derive(Debug, PartialEq, Eq)]
pub enum UpstreamDecision {
    /// `--upstream-manifest` was explicitly specified.
    Explicit(String),
    /// No upstream manifest should be used (local-only flow).
    LocalOnly,
    /// Detection succeeded and user confirmed; contains the composed reference.
    UseDetected(String),
}

// ---------------------------------------------------------------------------
// Internal deserialization types
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct RepoMeta {
    fork: bool,
    parent: Option<RepoRef>,
    template_repository: Option<RepoRef>,
}

#[derive(serde::Deserialize)]
struct RepoRef {
    full_name: String,
    default_branch: String,
}

// ---------------------------------------------------------------------------
// Detection functions
// ---------------------------------------------------------------------------

/// Return the current repository in `owner/name` format.
///
/// Checks `GITHUB_REPOSITORY` environment variable first (fast path for
/// GitHub Actions), then falls back to `gh repo view`.
///
/// # Errors
///
/// Returns an error when the `gh` CLI call fails or returns a non-zero exit
/// code.
#[allow(clippy::module_name_repetitions)]
pub fn detect_current_repo(runner: &dyn GhRunner) -> anyhow::Result<String> {
    if let Ok(v) = std::env::var("GITHUB_REPOSITORY")
        && !v.is_empty()
    {
        return Ok(v);
    }

    let out = run_checked(
        runner,
        &[
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "--jq",
            ".nameWithOwner",
        ],
        None,
        "gh repo view",
    )?;

    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Query GitHub and return the upstream parent of `current_repo`, if any.
///
/// Priority: fork parent (`parent` field) takes precedence over template
/// (`template_repository`).  Returns `Ok(None)` when neither is present.
///
/// # Errors
///
/// Returns an error when the `gh api` call fails, returns a non-zero exit
/// code, or the response cannot be parsed.
#[allow(clippy::module_name_repetitions)]
pub fn detect_upstream_parent(
    runner: &dyn GhRunner,
    current_repo: &str,
) -> anyhow::Result<Option<UpstreamParent>> {
    let url = format!("repos/{current_repo}");
    let out = run_checked(runner, &["api", &url], None, &format!("GET {url}"))?;

    let meta: RepoMeta =
        serde_json::from_slice(&out.stdout).context("failed to parse repository metadata JSON")?;

    // Fork parent takes priority over template.
    if let Some(p) = meta.fork.then_some(meta.parent).flatten() {
        return Ok(Some(UpstreamParent {
            full_name: p.full_name,
            default_branch: p.default_branch,
            source: ParentSource::Fork,
        }));
    }

    if let Some(t) = meta.template_repository {
        return Ok(Some(UpstreamParent {
            full_name: t.full_name,
            default_branch: t.default_branch,
            source: ParentSource::Template,
        }));
    }

    Ok(None)
}

// ---------------------------------------------------------------------------
// Pure decision logic (unit-testable without I/O)
// ---------------------------------------------------------------------------

/// Decide which upstream manifest reference to use.
///
/// - `explicit` — value of `--upstream-manifest` CLI flag.
/// - `yes` — `--yes` flag; skip detection prompt.
/// - `is_tty` — whether stdin is a terminal.
/// - `detected` — result of auto-detection (may be `None`).
/// - `manifest_path` — local manifest path used to compose the suggestion.
/// - `confirm` — closure called with the suggested reference string; returns
///   `true` when the user accepts.
pub fn decide_upstream_manifest(
    explicit: Option<&str>,
    yes: bool,
    is_tty: bool,
    detected: Option<&UpstreamParent>,
    manifest_path: &str,
    confirm: &dyn Fn(&str) -> bool,
) -> UpstreamDecision {
    if let Some(s) = explicit {
        return UpstreamDecision::Explicit(s.to_owned());
    }

    if yes || !is_tty {
        return UpstreamDecision::LocalOnly;
    }

    let Some(parent) = detected else {
        return UpstreamDecision::LocalOnly;
    };

    let suggested = format!(
        "{}@{}:{}",
        parent.full_name, parent.default_branch, manifest_path
    );

    if confirm(&suggested) {
        UpstreamDecision::UseDetected(suggested)
    } else {
        UpstreamDecision::LocalOnly
    }
}

// ---------------------------------------------------------------------------
// I/O orchestration (coverage(off) — thin shell around pure logic)
// ---------------------------------------------------------------------------

/// Try to detect parent; warn and return `None` on any error.
fn try_detect_parent(runner: &dyn GhRunner) -> Option<UpstreamParent> {
    let repo = match detect_current_repo(runner) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("could not determine current repository: {e:#}");
            return None;
        }
    };

    match detect_upstream_parent(runner, &repo) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("could not detect upstream parent for '{repo}': {e:#}");
            None
        }
    }
}

/// Compute the effective `--upstream-manifest` value, running detection and
/// prompting the user when appropriate.
///
/// Returns `Some(reference)` when an upstream should be used, `None` for
/// local-only mode.
///
/// # Coverage
///
/// Excluded from coverage because it performs I/O (spawns `gh`, shows a
/// `dialoguer` prompt).
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn resolve_effective_upstream(
    explicit: Option<&str>,
    yes: bool,
    manifest_path: &std::path::Path,
    runner: &dyn GhRunner,
) -> Option<String> {
    use std::io::IsTerminal as _;

    // Fast paths that require no detection.
    if let Some(s) = explicit {
        return Some(s.to_owned());
    }
    if yes || !std::io::stdin().is_terminal() {
        return None;
    }

    let parent = try_detect_parent(runner)?;
    let manifest_path_str = manifest_path.to_string_lossy();

    let decision = decide_upstream_manifest(
        None,
        false,
        true,
        Some(&parent),
        &manifest_path_str,
        &|suggested| {
            let prompt = format!(
                "Detected {} parent: {}@{}\nUse {} as upstream-manifest?",
                parent.source.label(),
                parent.full_name,
                parent.default_branch,
                suggested,
            );
            dialoguer::Confirm::new()
                .with_prompt(prompt)
                .default(false)
                .interact()
                .unwrap_or(false)
        },
    );

    match decision {
        UpstreamDecision::UseDetected(s) => Some(s),
        _ => None,
    }
}

/// Compute a default hint for `--repo` in `init` commands.
///
/// - For `--upstream` mode (`for_downstream = false`): returns the current
///   repo's own name.
/// - For `--downstream` mode (`for_downstream = true`): returns the parent
///   repo's `full_name`.
///
/// Returns `None` when detection fails or no parent is found (for downstream),
/// or when stdin is not a TTY.
#[allow(clippy::module_name_repetitions)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn detect_repo_hint(runner: &dyn GhRunner, for_downstream: bool) -> Option<String> {
    use std::io::IsTerminal as _;

    if !std::io::stdin().is_terminal() {
        return None;
    }

    if for_downstream {
        try_detect_parent(runner).map(|p| p.full_name)
    } else {
        detect_current_repo(runner).ok()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::panic)]

    use std::collections::VecDeque;
    use std::sync::Mutex;

    use crate::sync::runner::{GhOutput, GhRunner};

    use super::*;

    // -----------------------------------------------------------------------
    // Minimal MockGhRunner for detect.rs tests
    // -----------------------------------------------------------------------

    struct MockGhRunner {
        queue: Mutex<VecDeque<GhOutput>>,
    }

    impl MockGhRunner {
        fn new(responses: Vec<GhOutput>) -> Self {
            Self {
                queue: Mutex::new(responses.into()),
            }
        }

        fn ok(stdout: impl Into<Vec<u8>>) -> Self {
            Self::new(vec![GhOutput {
                exit_code: Some(0),
                stdout: stdout.into(),
                stderr: vec![],
            }])
        }

        fn fail(stderr: impl Into<Vec<u8>>) -> Self {
            Self::new(vec![GhOutput {
                exit_code: Some(1),
                stdout: vec![],
                stderr: stderr.into(),
            }])
        }
    }

    impl GhRunner for MockGhRunner {
        fn run(&self, _args: &[&str], _stdin: Option<&[u8]>) -> anyhow::Result<GhOutput> {
            self.queue
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("MockGhRunner: no more responses queued"))
        }
    }

    fn fork_json(parent_full_name: &str, parent_default_branch: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "fork": true,
            "parent": {
                "full_name": parent_full_name,
                "default_branch": parent_default_branch,
            },
            "template_repository": null
        }))
        .unwrap()
    }

    fn template_json(tmpl_full_name: &str, tmpl_default_branch: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "fork": false,
            "parent": null,
            "template_repository": {
                "full_name": tmpl_full_name,
                "default_branch": tmpl_default_branch,
            }
        }))
        .unwrap()
    }

    fn no_parent_json() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "fork": false,
            "parent": null,
            "template_repository": null
        }))
        .unwrap()
    }

    // -----------------------------------------------------------------------
    // detect_upstream_parent
    // -----------------------------------------------------------------------

    #[test]
    fn detects_fork_parent() {
        let runner = MockGhRunner::ok(fork_json("upstream/repo", "main"));
        let result = detect_upstream_parent(&runner, "owner/fork").unwrap();
        let parent = result.unwrap();
        assert_eq!(parent.full_name, "upstream/repo");
        assert_eq!(parent.default_branch, "main");
        assert_eq!(parent.source, ParentSource::Fork);
    }

    #[test]
    fn detects_template_parent() {
        let runner = MockGhRunner::ok(template_json("tmpl/repo", "main"));
        let result = detect_upstream_parent(&runner, "owner/clone").unwrap();
        let parent = result.unwrap();
        assert_eq!(parent.full_name, "tmpl/repo");
        assert_eq!(parent.default_branch, "main");
        assert_eq!(parent.source, ParentSource::Template);
    }

    #[test]
    fn returns_none_when_no_parent() {
        let runner = MockGhRunner::ok(no_parent_json());
        let result = detect_upstream_parent(&runner, "owner/standalone").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn fork_true_but_null_parent_falls_back_to_template() {
        let json = serde_json::to_vec(&serde_json::json!({
            "fork": true,
            "parent": null,
            "template_repository": {
                "full_name": "tmpl/repo",
                "default_branch": "main",
            }
        }))
        .unwrap();
        let runner = MockGhRunner::ok(json);
        let result = detect_upstream_parent(&runner, "owner/fork").unwrap();
        let parent = result.unwrap();
        assert_eq!(parent.full_name, "tmpl/repo");
        assert_eq!(parent.source, ParentSource::Template);
    }

    #[test]
    fn fork_true_both_null_returns_none() {
        let json = serde_json::to_vec(&serde_json::json!({
            "fork": true,
            "parent": null,
            "template_repository": null
        }))
        .unwrap();
        let runner = MockGhRunner::ok(json);
        let result = detect_upstream_parent(&runner, "owner/fork").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn gh_api_non_zero_returns_error() {
        let runner = MockGhRunner::fail(b"HTTP 404: Not Found".as_ref());
        let result = detect_upstream_parent(&runner, "owner/repo");
        assert!(result.is_err());
    }

    #[test]
    fn invalid_json_returns_error() {
        let runner = MockGhRunner::ok(b"not-json".as_ref());
        let result = detect_upstream_parent(&runner, "owner/repo");
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // decide_upstream_manifest (pure function — no I/O)
    // -----------------------------------------------------------------------

    fn sample_parent() -> UpstreamParent {
        UpstreamParent {
            full_name: String::from("upstream/repo"),
            default_branch: String::from("main"),
            source: ParentSource::Template,
        }
    }

    #[test]
    fn explicit_takes_priority() {
        let decision = decide_upstream_manifest(
            Some("explicit/repo@main:config.yaml"),
            false,
            true,
            Some(&sample_parent()),
            "config.yaml",
            &|_| panic!("should not be called"),
        );
        assert_eq!(
            decision,
            UpstreamDecision::Explicit(String::from("explicit/repo@main:config.yaml"))
        );
    }

    #[test]
    fn yes_flag_gives_local_only() {
        let decision = decide_upstream_manifest(
            None,
            true,
            true,
            Some(&sample_parent()),
            "config.yaml",
            &|_| panic!("should not be called"),
        );
        assert_eq!(decision, UpstreamDecision::LocalOnly);
    }

    #[test]
    fn non_tty_gives_local_only() {
        let decision = decide_upstream_manifest(
            None,
            false,
            false,
            Some(&sample_parent()),
            "config.yaml",
            &|_| panic!("should not be called"),
        );
        assert_eq!(decision, UpstreamDecision::LocalOnly);
    }

    #[test]
    fn no_detected_parent_gives_local_only() {
        let decision = decide_upstream_manifest(None, false, true, None, "config.yaml", &|_| {
            panic!("should not be called")
        });
        assert_eq!(decision, UpstreamDecision::LocalOnly);
    }

    #[test]
    fn user_confirms_gives_use_detected() {
        let decision = decide_upstream_manifest(
            None,
            false,
            true,
            Some(&sample_parent()),
            ".github/gh-sync/config.yaml",
            &|_| true,
        );
        assert_eq!(
            decision,
            UpstreamDecision::UseDetected(String::from(
                "upstream/repo@main:.github/gh-sync/config.yaml"
            ))
        );
    }

    #[test]
    fn user_declines_gives_local_only() {
        let decision = decide_upstream_manifest(
            None,
            false,
            true,
            Some(&sample_parent()),
            ".github/gh-sync/config.yaml",
            &|_| false,
        );
        assert_eq!(decision, UpstreamDecision::LocalOnly);
    }
}
