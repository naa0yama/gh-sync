use std::path::Path;

use super::StrategyResult;
use super::markers::{merge_marker_blocks, strip_marker_blocks};

// ---------------------------------------------------------------------------
// PatchOutput
// ---------------------------------------------------------------------------

/// Output of running the `patch` binary against upstream content.
#[derive(Debug)]
#[allow(clippy::module_name_repetitions)] // "PatchOutput" in module "patch" is intentional
pub enum PatchOutput {
    /// Patch applied cleanly; the resulting bytes are returned.
    Patched(Vec<u8>),
    /// One or more hunks failed to apply — the file was NOT modified.
    Conflict(String),
}

// ---------------------------------------------------------------------------
// PatchRunner trait
// ---------------------------------------------------------------------------

/// Abstracts the `patch` CLI call so the strategy logic can be tested without
/// spawning an actual subprocess.
///
/// The production implementation (`RealPatchRunner`) lives in the `gh-sync`
/// binary crate to keep this engine crate free of process-spawning I/O.
#[allow(clippy::module_name_repetitions)] // "PatchRunner" in module "patch" is intentional
pub trait PatchRunner {
    /// Apply `patch_file` to `upstream` bytes and return the outcome.
    ///
    /// # Errors
    ///
    /// Returns an error when the `patch` binary cannot be spawned or an
    /// unexpected exit code (≥ 2) is returned.
    fn apply_patch(&self, upstream: &[u8], patch_file: &Path) -> anyhow::Result<PatchOutput>;
}

// ---------------------------------------------------------------------------
// apply
// ---------------------------------------------------------------------------

/// Apply the `patch` strategy.
///
/// Applies `patch_file` to `upstream` content, then compares the result with
/// `local` to determine whether a write is needed.
///
/// When `preserve_markers` is `true`, marker blocks enclosed by
/// `gh-sync:keep-start` / `gh-sync:keep-end` comments are stripped from
/// `local` before comparison so they are excluded from drift detection.
/// If the patched result differs from the stripped local, the marker blocks
/// are merged back into the patched content before it is returned.
pub fn apply(
    upstream: &[u8],
    local: Option<&[u8]>,
    patch_file: &Path,
    runner: &dyn PatchRunner,
    preserve_markers: bool,
) -> StrategyResult {
    if preserve_markers {
        let upstream_stripped = match strip_marker_blocks(upstream) {
            Ok((s, _)) => s,
            Err(e) => {
                return StrategyResult::Error(format!("invalid marker block (upstream): {e}"));
            }
        };
        let patched = match runner.apply_patch(&upstream_stripped, patch_file) {
            Ok(PatchOutput::Patched(p)) => p,
            Ok(PatchOutput::Conflict(message)) => return StrategyResult::Conflict { message },
            Err(e) => return StrategyResult::Error(e.to_string()),
        };
        let (local_stripped, marker_blocks) = match local.map(strip_marker_blocks).transpose() {
            Ok(Some((s, b))) => (Some(s), b),
            Ok(None) => (None, vec![]),
            Err(e) => return StrategyResult::Error(format!("invalid marker block (local): {e}")),
        };
        if local_stripped.as_deref() == Some(patched.as_slice()) {
            StrategyResult::Unchanged
        } else {
            StrategyResult::Changed {
                content: merge_marker_blocks(&patched, &marker_blocks),
            }
        }
    } else {
        let patched = match runner.apply_patch(upstream, patch_file) {
            Ok(PatchOutput::Patched(p)) => p,
            Ok(PatchOutput::Conflict(message)) => return StrategyResult::Conflict { message },
            Err(e) => return StrategyResult::Error(e.to_string()),
        };
        if local == Some(patched.as_slice()) {
            StrategyResult::Unchanged
        } else {
            StrategyResult::Changed { content: patched }
        }
    }
}

// ---------------------------------------------------------------------------
// Test support
// ---------------------------------------------------------------------------

/// Mock implementations for use in tests that consume [`PatchRunner`].
#[cfg(any(test, feature = "testing"))]
pub mod testing {
    #![allow(missing_docs)]
    #![allow(clippy::missing_docs_in_private_items)]
    #![allow(missing_debug_implementations)]
    #![allow(clippy::must_use_candidate)]

    use std::path::Path;

    use super::{PatchOutput, PatchRunner};

    /// In-memory mock for use in tests that exercise code consuming
    /// [`PatchRunner`].
    pub struct MockPatchRunner {
        /// Callback invoked by [`PatchRunner::apply_patch`].
        pub result: Box<dyn Fn() -> anyhow::Result<PatchOutput> + Send + Sync>,
    }

    impl MockPatchRunner {
        /// Create a mock that always returns a successful patch with the given content.
        pub fn success(content: Vec<u8>) -> Self {
            Self {
                result: Box::new(move || Ok(PatchOutput::Patched(content.clone()))),
            }
        }

        /// Create a mock that always returns a conflict with the given message.
        pub fn conflict(msg: &'static str) -> Self {
            Self {
                result: Box::new(move || Ok(PatchOutput::Conflict(msg.to_owned()))),
            }
        }

        /// Create a mock that always returns an error with the given message.
        pub fn error(msg: &'static str) -> Self {
            Self {
                result: Box::new(move || Err(anyhow::anyhow!(msg))),
            }
        }
    }

    impl PatchRunner for MockPatchRunner {
        fn apply_patch(&self, _upstream: &[u8], _patch_file: &Path) -> anyhow::Result<PatchOutput> {
            (self.result)()
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

    use super::testing::MockPatchRunner;
    use super::*;

    // ------------------------------------------------------------------
    // apply() — mock runner
    // ------------------------------------------------------------------

    #[test]
    fn patched_differs_from_local_returns_changed() {
        // Arrange
        let runner = MockPatchRunner::success(b"patched\n".to_vec());

        // Act
        let result = apply(
            b"upstream\n",
            Some(b"old\n"),
            Path::new("x.patch"),
            &runner,
            false,
        );

        // Assert
        assert!(
            matches!(result, StrategyResult::Changed { ref content } if content == b"patched\n"),
            "expected Changed when patched result differs from local"
        );
    }

    #[test]
    fn patched_matches_local_returns_unchanged() {
        // Arrange
        let runner = MockPatchRunner::success(b"patched\n".to_vec());

        // Act
        let result = apply(
            b"upstream\n",
            Some(b"patched\n"),
            Path::new("x.patch"),
            &runner,
            false,
        );

        // Assert
        assert!(
            matches!(result, StrategyResult::Unchanged),
            "expected Unchanged when patched result equals local"
        );
    }

    #[test]
    fn patched_local_none_returns_changed() {
        // Arrange
        let runner = MockPatchRunner::success(b"patched\n".to_vec());

        // Act
        let result = apply(b"upstream\n", None, Path::new("x.patch"), &runner, false);

        // Assert
        assert!(
            matches!(result, StrategyResult::Changed { ref content } if content == b"patched\n"),
            "expected Changed when local does not exist"
        );
    }

    #[test]
    fn conflict_returns_conflict() {
        // Arrange
        let runner = MockPatchRunner::conflict("hunk 1 failed to apply");

        // Act
        let result = apply(b"upstream\n", None, Path::new("x.patch"), &runner, false);

        // Assert
        assert!(
            matches!(result, StrategyResult::Conflict { ref message }
                if message == "hunk 1 failed to apply"),
            "expected Conflict when runner returns Conflict"
        );
    }

    #[test]
    fn runner_error_returns_error() {
        // Arrange
        let runner = MockPatchRunner::error("patch binary not found");

        // Act
        let result = apply(b"upstream\n", None, Path::new("x.patch"), &runner, false);

        // Assert
        assert!(
            matches!(result, StrategyResult::Error(ref msg) if msg.contains("patch binary not found")),
            "expected Error when runner returns Err"
        );
    }
}
