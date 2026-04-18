//! Upstream fetcher: re-exports trait/types from `gh-sync-engine`, production `GhFetcher` here.

// Re-export trait and pure types from engine.
#[allow(clippy::module_name_repetitions)]
pub use gh_sync_engine::upstream::{FetchResult, TreeEntry, UpstreamFetcher};

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
#[derive(Debug)]
pub struct GhFetcher;

impl UpstreamFetcher for GhFetcher {
    // NOTEST(io): thin wrapper around the `gh` CLI binary — exercised via
    // integration tests only; unit tests use MockUpstreamFetcher instead.
    #[cfg_attr(coverage_nightly, coverage(off))]
    fn fetch(&self, repo: &str, ref_: &str, path: &str) -> anyhow::Result<FetchResult> {
        use anyhow::Context as _;

        let url = format!("repos/{repo}/contents/{path}?ref={ref_}");
        let output = std::process::Command::new("gh")
            .args(["api", "-H", "Accept: application/vnd.github.raw", &url])
            .output()
            .context("failed to spawn `gh`")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("HTTP 404") {
                return Ok(FetchResult::NotFound);
            }
            anyhow::bail!("`gh api` failed: {stderr}");
        }

        Ok(FetchResult::Content(output.stdout))
    }

    // NOTEST(io): thin wrapper around the `gh` CLI binary — exercised via
    // integration tests only; unit tests use MockFetcher instead.
    #[cfg_attr(coverage_nightly, coverage(off))]
    fn resolve_tag_sha(&self, repo: &str, tag: &str) -> anyhow::Result<String> {
        use anyhow::Context as _;

        let url = format!("repos/{repo}/commits/{tag}");
        let output = std::process::Command::new("gh")
            .args(["api", &url, "--jq", ".sha"])
            .output()
            .context("failed to spawn `gh`")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("`gh api` failed: {stderr}");
        }

        let sha = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        Ok(sha)
    }

    // NOTEST(io): thin wrapper around the `gh` CLI binary — exercised via
    // integration tests only; unit tests use MockFetcher instead.
    #[cfg_attr(coverage_nightly, coverage(off))]
    fn list_all_files(&self, repo: &str, ref_: &str) -> anyhow::Result<Vec<TreeEntry>> {
        use anyhow::Context as _;

        let url = format!("repos/{repo}/git/trees/{ref_}?recursive=1");
        let output = std::process::Command::new("gh")
            .args(["api", &url])
            .output()
            .context("failed to spawn `gh`")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("`gh api` failed: {stderr}");
        }

        let response = serde_json::from_slice::<TreeResponse>(&output.stdout)
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
