//! Re-export `patch` strategy types from `gh-sync-engine`; `RealPatchRunner` stays here.

use std::path::Path;

// Re-export trait, types, and pure function from engine.
#[allow(unused_imports, clippy::module_name_repetitions)]
pub use gh_sync_engine::strategy::patch::{PatchOutput, PatchRunner, apply};

// ---------------------------------------------------------------------------
// RealPatchRunner
// ---------------------------------------------------------------------------

/// Production implementation: writes `upstream` to a temp file and calls the
/// system `patch` binary.
pub struct RealPatchRunner;

impl std::fmt::Debug for RealPatchRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RealPatchRunner").finish()
    }
}

impl PatchRunner for RealPatchRunner {
    #[cfg_attr(coverage_nightly, coverage(off))]
    fn apply_patch(&self, upstream: &[u8], patch_file: &Path) -> anyhow::Result<PatchOutput> {
        use anyhow::Context as _;

        let dir = tempfile::tempdir().context("failed to create temp dir")?;
        let orig_path = dir.path().join("original");
        let result_path = dir.path().join("result");

        std::fs::write(&orig_path, upstream).context("failed to write original content")?;

        let output = std::process::Command::new("patch")
            .args([
                "--no-backup-if-mismatch",
                "-o",
                result_path.to_str().context("non-UTF-8 result path")?,
                "-i",
                patch_file.to_str().context("non-UTF-8 patch file path")?,
                orig_path.to_str().context("non-UTF-8 original path")?,
            ])
            .output()
            .context("failed to spawn `patch`")?;

        match output.status.code() {
            Some(0) => {
                let content =
                    std::fs::read(&result_path).context("failed to read patched result")?;
                Ok(PatchOutput::Patched(content))
            }
            Some(1) => {
                let message = String::from_utf8_lossy(&output.stderr).into_owned();
                Ok(PatchOutput::Conflict(message))
            }
            // NOTEST(io): exit code ≥ 2 from `patch` indicates an invocation error —
            // unreachable with valid temp file paths in normal operation
            _ => {
                anyhow::bail!(
                    "`patch` exited with {}: {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr)
                )
            }
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

    use super::*;

    /// Simple unified diff that changes "hello" to "world".
    fn simple_patch() -> &'static str {
        "--- original\n+++ result\n@@ -1 +1 @@\n-hello\n+world\n"
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn real_runner_applies_clean_patch() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let patch_path = dir.path().join("test.patch");
        std::fs::write(&patch_path, simple_patch()).unwrap();

        // Act
        let result = RealPatchRunner
            .apply_patch(b"hello\n", &patch_path)
            .unwrap();

        // Assert
        assert!(
            matches!(result, PatchOutput::Patched(ref content) if content == b"world\n"),
            "expected Patched(world\\n)"
        );
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn real_runner_conflict_on_mismatched_content() {
        // Arrange — patch expects "hello" but upstream has different content
        let dir = tempfile::tempdir().unwrap();
        let patch_path = dir.path().join("bad.patch");
        std::fs::write(&patch_path, simple_patch()).unwrap();

        // Act
        let result = RealPatchRunner
            .apply_patch(b"completely different\n", &patch_path)
            .unwrap();

        // Assert
        assert!(
            matches!(result, PatchOutput::Conflict(_)),
            "expected Conflict on mismatch"
        );
    }
}
