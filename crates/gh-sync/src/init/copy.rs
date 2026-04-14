use crate::sync::upstream::{FetchResult, UpstreamFetcher};

/// Fetch the upstream's own gh-sync config and return its content
/// with the schema comment prepended.
///
/// # Errors
///
/// Returns an error when the upstream config cannot be fetched or is not
/// a valid YAML manifest.
pub fn fetch_upstream_config(
    fetcher: &dyn UpstreamFetcher,
    repo: &str,
    ref_: &str,
) -> anyhow::Result<String> {
    use anyhow::Context as _;

    match fetcher.fetch(repo, ref_, ".github/gh-sync/config.yaml")? {
        FetchResult::NotFound => anyhow::bail!(
            "upstream repository '{repo}' does not have a gh-sync config at \
             .github/gh-sync/config.yaml\n\
             hint: use `--select` to interactively generate a config instead"
        ),
        FetchResult::Content(bytes) => {
            let text =
                String::from_utf8(bytes).context("upstream config contains non-UTF-8 bytes")?;
            // Validate that it parses as a manifest before writing
            serde_yml::from_str::<crate::sync::manifest::Manifest>(&text)
                .context("upstream config is not a valid gh-sync manifest")?;
            Ok(format!("{}{text}", super::schema::comment()))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use crate::sync::upstream::testing::MockFetcher;

    use super::*;

    const VALID_CONFIG: &str =
        "upstream:\n  repo: owner/repo\nfiles:\n  - path: foo.txt\n    strategy: replace\n";

    #[test]
    fn returns_content_with_schema_comment() {
        let fetcher = MockFetcher::content(VALID_CONFIG.as_bytes().to_vec());
        let result = fetch_upstream_config(&fetcher, "owner/repo", "main").unwrap();
        assert!(result.starts_with("# yaml-language-server"));
        assert!(result.contains("upstream:"));
    }

    #[test]
    fn errors_on_not_found() {
        let fetcher = MockFetcher::not_found();
        let err = fetch_upstream_config(&fetcher, "owner/repo", "main").unwrap_err();
        assert!(err.to_string().contains("does not have a gh-sync config"));
        assert!(err.to_string().contains("--select"));
    }

    #[test]
    fn errors_on_invalid_yaml() {
        let fetcher = MockFetcher::content(b"not: valid: manifest: !!!".to_vec());
        let err = fetch_upstream_config(&fetcher, "owner/repo", "main").unwrap_err();
        assert!(err.to_string().contains("valid gh-sync manifest"));
    }
}
