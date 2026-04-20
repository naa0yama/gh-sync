//! Upstream fetcher: re-exports trait/types from `gh-sync-engine`, production `GhFetcher` here.

// Re-export trait and pure types from engine.
#[allow(clippy::module_name_repetitions)]
pub use gh_sync_engine::upstream::{FetchResult, TreeEntry, UpstreamFetcher};

use crate::sync::gh_error;
use crate::sync::runner::{GhRunner, SystemGhRunner, run_checked};

// ---------------------------------------------------------------------------
// TreeResponse (internal — only used by GhFetcher below)
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct TreeResponse {
    tree: Vec<TreeEntry>,
    truncated: bool,
}

// ---------------------------------------------------------------------------
// GhFetcher
// ---------------------------------------------------------------------------

/// Production implementation: calls `gh api` with a raw-content Accept header.
pub struct GhFetcher {
    runner: Box<dyn GhRunner>,
}

impl std::fmt::Debug for GhFetcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GhFetcher").finish_non_exhaustive()
    }
}

impl GhFetcher {
    /// Create with the real `gh` CLI.
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub fn new() -> Self {
        Self {
            runner: Box::new(SystemGhRunner),
        }
    }

    /// Create with an injected runner (for tests).
    #[cfg(test)]
    pub fn with_runner(runner: impl GhRunner + 'static) -> Self {
        Self {
            runner: Box::new(runner),
        }
    }
}

impl Default for GhFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl UpstreamFetcher for GhFetcher {
    // NOTEST(io): thin wrapper around the `gh` CLI binary — exercised via
    // integration tests only; unit tests use MockUpstreamFetcher instead.
    #[cfg_attr(coverage_nightly, coverage(off))]
    fn fetch(&self, repo: &str, ref_: &str, path: &str) -> anyhow::Result<FetchResult> {
        let url = format!("repos/{repo}/contents/{path}?ref={ref_}");
        let out = self.runner.run(
            &["api", "-H", "Accept: application/vnd.github.raw", &url],
            None,
        )?;

        if !out.success() {
            let api_err = gh_error::parse_from_streams(&out.stdout, &out.stderr);
            let status = api_err.as_ref().and_then(|e| e.status);
            let stderr_str = String::from_utf8_lossy(&out.stderr);
            // 2-stage: API status first, string fallback.
            if status == Some(404) || (status.is_none() && stderr_str.contains("HTTP 404")) {
                return Ok(FetchResult::NotFound);
            }
            let exit_code = out.exit_code;
            let stdout_str = String::from_utf8_lossy(&out.stdout);
            let op = format!("GET {url}");
            tracing::error!(%op, ?exit_code, stderr = %stderr_str, stdout = %stdout_str, ?api_err, "gh command failed");
            let stderr_summary = gh_error::truncate_tail(&stderr_str, 2048);
            anyhow::bail!("{op} failed (exit {exit_code:?}): {stderr_summary}");
        }

        Ok(FetchResult::Content(out.stdout))
    }

    // NOTEST(io): thin wrapper around the `gh` CLI binary — exercised via
    // integration tests only; unit tests use MockFetcher instead.
    #[cfg_attr(coverage_nightly, coverage(off))]
    fn resolve_tag_sha(&self, repo: &str, tag: &str) -> anyhow::Result<String> {
        let url = format!("repos/{repo}/commits/{tag}");
        let out = run_checked(
            self.runner.as_ref(),
            &["api", &url, "--jq", ".sha"],
            None,
            &format!("GET {url}"),
        )?;
        let sha = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        Ok(sha)
    }

    // NOTEST(io): thin wrapper around the `gh` CLI binary — exercised via
    // integration tests only; unit tests use MockFetcher instead.
    #[cfg_attr(coverage_nightly, coverage(off))]
    fn list_all_files(&self, repo: &str, ref_: &str) -> anyhow::Result<Vec<TreeEntry>> {
        use anyhow::Context as _;

        let url = format!("repos/{repo}/git/trees/{ref_}?recursive=1");
        let out = run_checked(
            self.runner.as_ref(),
            &["api", &url],
            None,
            &format!("GET {url}"),
        )?;

        let response = serde_json::from_slice::<TreeResponse>(&out.stdout)
            .context("failed to parse tree response JSON")?;

        if response.truncated {
            anyhow::bail!(
                "repository '{repo}' has too many files; the tree response was truncated"
            );
        }

        Ok(response
            .tree
            .into_iter()
            .filter(|e| e.type_ == "blob")
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::collections::VecDeque;
    use std::sync::Mutex;

    use crate::sync::runner::{GhOutput, GhRunner};

    use super::*;

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

        fn fail_with(stdout: impl Into<Vec<u8>>, stderr: impl Into<Vec<u8>>) -> Self {
            Self::new(vec![GhOutput {
                exit_code: Some(1),
                stdout: stdout.into(),
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

    fn tree_json(entries: &[(&str, &str)], truncated: bool) -> Vec<u8> {
        let tree: Vec<_> = entries
            .iter()
            .map(|(path, type_)| serde_json::json!({ "path": path, "type": type_ }))
            .collect();
        serde_json::to_vec(&serde_json::json!({ "tree": tree, "truncated": truncated })).unwrap()
    }

    // --- fetch tests ---

    #[test]
    fn fetch_success_returns_content() {
        let runner = MockGhRunner::ok(b"file content".as_ref());
        let fetcher = GhFetcher::with_runner(runner);
        let result = fetcher.fetch("owner/repo", "main", "README.md").unwrap();
        assert!(matches!(result, FetchResult::Content(ref c) if c == b"file content"));
    }

    #[test]
    fn fetch_404_via_api_status_returns_not_found() {
        let json_404 = serde_json::to_vec(&serde_json::json!({
            "message": "Not Found",
            "status": "404"
        }))
        .unwrap();
        let runner = MockGhRunner::fail_with(json_404, b"gh: Not Found (HTTP 404)".as_ref());
        let fetcher = GhFetcher::with_runner(runner);
        let result = fetcher.fetch("owner/repo", "main", "missing.md").unwrap();
        assert!(matches!(result, FetchResult::NotFound));
    }

    #[test]
    fn fetch_404_via_stderr_string_returns_not_found() {
        let runner = MockGhRunner::fail_with(vec![], b"HTTP 404 Not Found".as_ref());
        let fetcher = GhFetcher::with_runner(runner);
        let result = fetcher.fetch("owner/repo", "main", "missing.md").unwrap();
        assert!(matches!(result, FetchResult::NotFound));
    }

    #[test]
    fn fetch_non_404_error_returns_err() {
        let runner = MockGhRunner::fail_with(vec![], b"gh: server error (HTTP 500)".as_ref());
        let fetcher = GhFetcher::with_runner(runner);
        let result = fetcher.fetch("owner/repo", "main", "file.md");
        assert!(result.is_err());
    }

    // --- resolve_tag_sha tests ---

    #[test]
    fn resolve_tag_sha_returns_trimmed_sha() {
        let runner = MockGhRunner::ok(b"abc123def456\n".as_ref());
        let fetcher = GhFetcher::with_runner(runner);
        let sha = fetcher.resolve_tag_sha("owner/repo", "v1.0.0").unwrap();
        assert_eq!(sha, "abc123def456");
    }

    #[test]
    fn resolve_tag_sha_error_returns_err() {
        let runner = MockGhRunner::fail_with(vec![], b"gh: Not Found (HTTP 404)".as_ref());
        let fetcher = GhFetcher::with_runner(runner);
        let result = fetcher.resolve_tag_sha("owner/repo", "v9.9.9");
        assert!(result.is_err());
    }

    // --- list_all_files tests ---

    #[test]
    fn list_all_files_returns_only_blobs() {
        let json = tree_json(&[("src/main.rs", "blob"), ("src", "tree")], false);
        let runner = MockGhRunner::ok(json);
        let fetcher = GhFetcher::with_runner(runner);
        let files = fetcher.list_all_files("owner/repo", "main").unwrap();
        assert_eq!(files.len(), 1);
        let first = files.first().unwrap();
        assert_eq!(first.path, "src/main.rs");
    }

    #[test]
    fn list_all_files_truncated_returns_err() {
        let json = tree_json(&[("file.rs", "blob")], true);
        let runner = MockGhRunner::ok(json);
        let fetcher = GhFetcher::with_runner(runner);
        let result = fetcher.list_all_files("owner/repo", "main");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("truncated"));
    }

    #[test]
    fn list_all_files_api_error_returns_err() {
        let runner =
            MockGhRunner::fail_with(vec![], b"gh: internal server error (HTTP 500)".as_ref());
        let fetcher = GhFetcher::with_runner(runner);
        let result = fetcher.list_all_files("owner/repo", "main");
        assert!(result.is_err());
    }

    #[test]
    fn list_all_files_invalid_json_returns_err() {
        let runner = MockGhRunner::ok(b"not valid json".as_ref());
        let fetcher = GhFetcher::with_runner(runner);
        let result = fetcher.list_all_files("owner/repo", "main");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("parse tree response")
        );
    }
}
