// ---------------------------------------------------------------------------
// TreeEntry
// ---------------------------------------------------------------------------

/// A single entry returned by the GitHub Git Trees API.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TreeEntry {
    /// Path of the entry relative to the repository root.
    pub path: String,
    /// Entry type: `"blob"` (file) or `"tree"` (directory).
    #[serde(rename = "type")]
    pub type_: String,
}

// ---------------------------------------------------------------------------
// FetchResult
// ---------------------------------------------------------------------------

/// Result of fetching a single file from the upstream repository.
#[derive(Debug)]
pub enum FetchResult {
    /// The file was found and its raw bytes are returned.
    Content(Vec<u8>),
    /// The file does not exist in the upstream repository at the given ref.
    NotFound,
}

// ---------------------------------------------------------------------------
// UpstreamFetcher trait
// ---------------------------------------------------------------------------

/// Abstracts fetching files from an upstream GitHub repository.
///
/// The trait enables mock injection for unit tests; production code uses
/// `GhFetcher` from the `gh-sync` binary crate.
#[allow(clippy::module_name_repetitions)] // "UpstreamFetcher" in module "upstream" is intentional
pub trait UpstreamFetcher {
    /// Fetch the file at `path` from `repo` at git `ref_`.
    ///
    /// # Errors
    ///
    /// Returns an error when the `gh` CLI cannot be spawned or returns a
    /// non-404 failure status.
    fn fetch(&self, repo: &str, ref_: &str, path: &str) -> anyhow::Result<FetchResult>;

    /// Recursively list all files in `repo` at git `ref_` using the Git Trees
    /// API with `?recursive=1`.
    ///
    /// Returns only `"blob"` entries (i.e. files, no directories).
    ///
    /// # Errors
    ///
    /// Returns an error when the `gh` CLI cannot be spawned, returns a
    /// non-success status, the JSON response cannot be parsed, or the tree
    /// response is truncated (>100 000 entries).
    fn list_all_files(&self, repo: &str, ref_: &str) -> anyhow::Result<Vec<TreeEntry>>;
}

// ---------------------------------------------------------------------------
// Test support
// ---------------------------------------------------------------------------

/// Mock implementations for use in tests that consume [`UpstreamFetcher`].
#[cfg(any(test, feature = "testing"))]
pub mod testing {
    #![allow(clippy::type_complexity)]
    #![allow(missing_docs)]
    #![allow(clippy::missing_docs_in_private_items)]
    #![allow(missing_debug_implementations)]
    #![allow(clippy::must_use_candidate)]

    use super::{FetchResult, UpstreamFetcher};

    /// Closure type for the single-file fetch callback.
    pub type FetchFn = Box<dyn Fn(&str, &str, &str) -> anyhow::Result<FetchResult> + Send + Sync>;
    /// Closure type for the list-all-files callback.
    pub type AllFilesFn =
        Box<dyn Fn(&str, &str) -> anyhow::Result<Vec<super::TreeEntry>> + Send + Sync>;

    /// In-memory mock for use in tests that exercise code consuming
    /// [`UpstreamFetcher`].
    pub struct MockFetcher {
        /// Callback invoked by [`UpstreamFetcher::fetch`].
        pub result: FetchFn,
        /// Callback invoked by [`UpstreamFetcher::list_all_files`].
        pub all_files_result: AllFilesFn,
    }

    impl MockFetcher {
        /// Create a mock that always returns the given `bytes` as content.
        pub fn content(bytes: Vec<u8>) -> Self {
            Self {
                result: Box::new(move |_, _, _| Ok(FetchResult::Content(bytes.clone()))),
                all_files_result: Box::new(|_, _| Ok(vec![])),
            }
        }

        /// Create a mock that always returns `NotFound`.
        pub fn not_found() -> Self {
            Self {
                result: Box::new(|_, _, _| Ok(FetchResult::NotFound)),
                all_files_result: Box::new(|_, _| Ok(vec![])),
            }
        }

        /// Create a mock that always returns the given error message.
        pub fn error(msg: &'static str) -> Self {
            Self {
                result: Box::new(move |_, _, _| Err(anyhow::anyhow!(msg))),
                all_files_result: Box::new(|_, _| Ok(vec![])),
            }
        }

        /// Create a mock whose `list_all_files` returns the given entries.
        #[allow(dead_code)]
        pub fn with_all_files(entries: Vec<super::TreeEntry>) -> Self {
            Self {
                result: Box::new(|_, _, _| Ok(FetchResult::NotFound)),
                all_files_result: Box::new(move |_, _| Ok(entries.clone())),
            }
        }
    }

    impl UpstreamFetcher for MockFetcher {
        fn fetch(&self, repo: &str, ref_: &str, path: &str) -> anyhow::Result<FetchResult> {
            (self.result)(repo, ref_, path)
        }

        fn list_all_files(&self, repo: &str, ref_: &str) -> anyhow::Result<Vec<super::TreeEntry>> {
            (self.all_files_result)(repo, ref_)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::indexing_slicing)]

    use super::testing::MockFetcher;
    use super::*;

    #[test]
    fn mock_fetcher_returns_content() {
        // Arrange
        let fetcher = MockFetcher::content(b"file content".to_vec());

        // Act
        let result = fetcher.fetch("owner/repo", "main", "path/to/file").unwrap();

        // Assert
        assert!(
            matches!(result, FetchResult::Content(ref bytes) if bytes == b"file content"),
            "expected Content variant"
        );
    }

    #[test]
    fn mock_fetcher_returns_not_found() {
        // Arrange
        let fetcher = MockFetcher::not_found();

        // Act
        let result = fetcher.fetch("owner/repo", "main", "missing/file").unwrap();

        // Assert
        assert!(
            matches!(result, FetchResult::NotFound),
            "expected NotFound variant"
        );
    }

    #[test]
    fn mock_fetcher_propagates_error() {
        // Arrange
        let fetcher = MockFetcher::error("network error");

        // Act
        let err = fetcher.fetch("owner/repo", "main", "any/file").unwrap_err();

        // Assert
        assert!(
            err.to_string().contains("network error"),
            "expected error message"
        );
    }

    #[test]
    fn mock_fetcher_with_all_files_returns_entries() {
        let entries = vec![
            super::TreeEntry {
                path: String::from("src/main.rs"),
                type_: String::from("blob"),
            },
            super::TreeEntry {
                path: String::from("src/lib.rs"),
                type_: String::from("blob"),
            },
        ];
        let fetcher = MockFetcher::with_all_files(entries);
        let result = fetcher.list_all_files("owner/repo", "main").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].path, "src/main.rs");
        assert_eq!(result[1].path, "src/lib.rs");
    }

    #[test]
    fn mock_fetcher_list_all_files_default_is_empty() {
        let fetcher = MockFetcher::content(b"data".to_vec());
        let result = fetcher.list_all_files("owner/repo", "main").unwrap();
        assert!(result.is_empty());
    }
}
