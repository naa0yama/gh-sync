//! Parsing and fetching of `--upstream-manifest` references.
//!
//! The reference format is `owner/repo@ref:path`, where `@ref` may be omitted
//! (defaults to `"HEAD"`) but `:path` is always required.

use anyhow::Context as _;
use gh_sync_manifest::{Manifest, merge_overlay};

use crate::sync::upstream::UpstreamFetcher;

// ---------------------------------------------------------------------------
// Reference type
// ---------------------------------------------------------------------------

/// A parsed `--upstream-manifest` value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)]
pub struct UpstreamManifestRef {
    /// `owner/name` repository.
    pub repo: String,
    /// Git ref (branch, tag, or commit SHA). Defaults to `"HEAD"`.
    pub ref_: String,
    /// Path to the manifest YAML inside the repository.
    pub path: String,
}

impl UpstreamManifestRef {
    /// Parse `owner/repo@ref:path` or `owner/repo:path` (ref defaults to `"HEAD"`).
    ///
    /// # Errors
    ///
    /// Returns an error when the format is invalid.
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        // Split on the last `:` to get the path.
        let Some((repo_and_ref, path)) = s.rsplit_once(':') else {
            anyhow::bail!(
                "invalid upstream-manifest '{s}': missing ':path' \
                 (expected owner/repo@ref:path)"
            );
        };

        if path.is_empty() {
            anyhow::bail!(
                "invalid upstream-manifest '{s}': path must not be empty \
                 (expected owner/repo@ref:path)"
            );
        }

        // Split on `@` to separate the repo from the ref.
        let (repo, ref_) = if let Some((r, rf)) = repo_and_ref.split_once('@') {
            if rf.is_empty() {
                anyhow::bail!("invalid upstream-manifest '{s}': ref after '@' must not be empty");
            }
            (r.to_owned(), rf.to_owned())
        } else {
            (repo_and_ref.to_owned(), String::from("HEAD"))
        };

        // Validate owner/name format (same rule as manifest upstream.repo).
        if !is_valid_repo(&repo) {
            anyhow::bail!(
                "invalid upstream-manifest '{s}': repository '{repo}' must be \
                 owner/name format (e.g. naa0yama/boilerplate-rust)"
            );
        }

        Ok(Self {
            repo,
            ref_,
            path: path.to_owned(),
        })
    }
}

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

// ---------------------------------------------------------------------------
// Fetch + merge
// ---------------------------------------------------------------------------

/// Fetch the upstream manifest and merge it with an optional local manifest.
///
/// Steps:
/// 1. Fetch `ref.path` from `ref.repo@ref.ref_` via `fetcher`.
/// 2. Parse the fetched YAML as a [`Manifest`].
/// 3. Call [`merge_overlay`] with `upstream` and `local`.
///
/// # Errors
///
/// Returns an error when:
/// - The fetch fails or returns `NotFound`.
/// - The fetched content cannot be parsed as a valid manifest YAML.
pub fn fetch_and_merge(
    ref_: &UpstreamManifestRef,
    fetcher: &dyn UpstreamFetcher,
    local: Option<Manifest>,
) -> anyhow::Result<Manifest> {
    use gh_sync_engine::upstream::FetchResult;

    let bytes = match fetcher
        .fetch(&ref_.repo, &ref_.ref_, &ref_.path)
        .with_context(|| {
            format!(
                "failed to fetch upstream manifest '{}@{}:{}'",
                ref_.repo, ref_.ref_, ref_.path
            )
        })? {
        FetchResult::Content(b) => b,
        FetchResult::NotFound => {
            anyhow::bail!(
                "upstream manifest not found: '{}@{}:{}'",
                ref_.repo,
                ref_.ref_,
                ref_.path
            );
        }
    };

    let yaml = String::from_utf8(bytes).with_context(|| {
        format!(
            "upstream manifest '{}@{}:{}' is not valid UTF-8",
            ref_.repo, ref_.ref_, ref_.path
        )
    })?;

    let upstream: Manifest = serde_yml::from_str(&yaml).with_context(|| {
        format!(
            "failed to parse upstream manifest '{}@{}:{}'",
            ref_.repo, ref_.ref_, ref_.path
        )
    })?;

    Ok(merge_overlay(upstream, local))
}

// ---------------------------------------------------------------------------
// Convenience resolver
// ---------------------------------------------------------------------------

/// Resolve the effective manifest for a sync command.
///
/// - When `upstream_ref_str` is `Some`: parse and fetch the upstream manifest,
///   then merge with the local manifest (if the local file exists at
///   `local_path`).
/// - When `upstream_ref_str` is `None`: load the manifest from `local_path`
///   (must exist).
///
/// # Errors
///
/// Returns an error when:
/// - `upstream_ref_str` cannot be parsed.
/// - The upstream manifest cannot be fetched or parsed.
/// - `upstream_ref_str` is `None` and `local_path` cannot be read.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn resolve(
    upstream_ref_str: Option<&str>,
    local_path: &std::path::Path,
    fetcher: &dyn UpstreamFetcher,
) -> anyhow::Result<Manifest> {
    if let Some(ref_str) = upstream_ref_str {
        let ref_ = UpstreamManifestRef::parse(ref_str)
            .with_context(|| format!("failed to parse upstream-manifest '{ref_str}'"))?;

        // Load the local manifest as an optional overlay.
        let local = if local_path.exists() {
            Some(Manifest::load(local_path).with_context(|| {
                format!("failed to load local manifest '{}'", local_path.display())
            })?)
        } else {
            None
        };

        fetch_and_merge(&ref_, fetcher, local)
    } else {
        Manifest::load(local_path)
            .with_context(|| format!("failed to load manifest '{}'", local_path.display()))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::indexing_slicing)]

    use super::*;

    // --- UpstreamManifestRef::parse ------------------------------------------

    #[test]
    fn parse_full_form() {
        let r = UpstreamManifestRef::parse(
            "naa0yama/boilerplate-rust@main:.github/gh-sync/config.yaml",
        )
        .unwrap();
        assert_eq!(r.repo, "naa0yama/boilerplate-rust");
        assert_eq!(r.ref_, "main");
        assert_eq!(r.path, ".github/gh-sync/config.yaml");
    }

    #[test]
    fn parse_without_ref_defaults_to_head() {
        let r = UpstreamManifestRef::parse("owner/repo:config.yaml").unwrap();
        assert_eq!(r.repo, "owner/repo");
        assert_eq!(r.ref_, "HEAD");
        assert_eq!(r.path, "config.yaml");
    }

    #[test]
    fn parse_nested_path() {
        let r =
            UpstreamManifestRef::parse("owner/repo@v1.0.0:.github/gh-sync/config.yaml").unwrap();
        assert_eq!(r.ref_, "v1.0.0");
        assert_eq!(r.path, ".github/gh-sync/config.yaml");
    }

    #[test]
    fn parse_missing_path_separator_errors() {
        assert!(UpstreamManifestRef::parse("owner/repo@main").is_err());
    }

    #[test]
    fn parse_empty_path_errors() {
        assert!(UpstreamManifestRef::parse("owner/repo@main:").is_err());
    }

    #[test]
    fn parse_empty_ref_errors() {
        assert!(UpstreamManifestRef::parse("owner/repo@:path").is_err());
    }

    #[test]
    fn parse_invalid_repo_format_errors() {
        assert!(UpstreamManifestRef::parse("notarepo@main:path").is_err());
    }

    #[test]
    fn parse_repo_with_double_slash_errors() {
        assert!(UpstreamManifestRef::parse("a/b/c@main:path").is_err());
    }

    // --- fetch_and_merge --------------------------------------------------

    #[test]
    fn fetch_and_merge_no_local() {
        use gh_sync_engine::upstream::testing::MockFetcher;

        let yaml =
            b"upstream:\n  repo: owner/repo\nfiles:\n  - path: foo.txt\n    strategy: replace\n"
                .to_vec();
        let fetcher = MockFetcher::content(yaml);
        let ref_ = UpstreamManifestRef {
            repo: String::from("owner/repo"),
            ref_: String::from("main"),
            path: String::from("config.yaml"),
        };

        let m = fetch_and_merge(&ref_, &fetcher, None).unwrap();
        assert_eq!(m.files.len(), 1);
        assert_eq!(m.files[0].path, "foo.txt");
    }

    #[test]
    fn fetch_not_found_returns_error() {
        use gh_sync_engine::upstream::testing::MockFetcher;

        let fetcher = MockFetcher::not_found();
        let ref_ = UpstreamManifestRef {
            repo: String::from("owner/repo"),
            ref_: String::from("main"),
            path: String::from("config.yaml"),
        };

        assert!(fetch_and_merge(&ref_, &fetcher, None).is_err());
    }

    #[test]
    fn fetch_error_propagates() {
        use gh_sync_engine::upstream::testing::MockFetcher;

        let fetcher = MockFetcher::error("network down");
        let ref_ = UpstreamManifestRef {
            repo: String::from("owner/repo"),
            ref_: String::from("main"),
            path: String::from("config.yaml"),
        };

        assert!(fetch_and_merge(&ref_, &fetcher, None).is_err());
    }
}
