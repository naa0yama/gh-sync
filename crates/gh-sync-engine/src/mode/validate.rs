//! `--validate` mode: offline manifest schema + reference validation.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use gh_sync_manifest::{self as manifest, Manifest, Strategy};

use crate::output::StatusTag;

/// Run manifest validation and write results to `w`.
///
/// Performs Stage 1 (schema) and Stage 2 (local file references) validation.
/// Returns `ExitCode::SUCCESS` when the manifest is valid, `ExitCode::FAILURE`
/// when one or more errors are found.
///
/// # Errors
/// Propagates I/O errors from `w`.
pub fn run(manifest_path: &Path, repo_root: &Path, w: &mut dyn Write) -> std::io::Result<ExitCode> {
    // Load the manifest
    let m = match Manifest::load(manifest_path) {
        Ok(m) => m,
        Err(e) => {
            writeln!(w, "{}  {e}", StatusTag::Fail.styled())?;
            return Ok(ExitCode::FAILURE);
        }
    };

    // Stage 1 — schema validation
    let schema_ok = match manifest::validate_schema(&m) {
        Ok(()) => {
            writeln!(w, "{}  YAML syntax valid", StatusTag::Ok.styled())?;
            writeln!(
                w,
                "{}  upstream.repo: {}",
                StatusTag::Ok.styled(),
                m.upstream.repo
            )?;
            true
        }
        Err(e) => {
            writeln!(
                w,
                "{}  schema validation failed: {e}",
                StatusTag::Fail.styled()
            )?;
            false
        }
    };

    // Per-file summary (only when schema passed, so files are trustworthy)
    if schema_ok {
        for (i, rule) in m.files.iter().enumerate() {
            let detail = if rule.strategy == Strategy::Patch {
                let patch_path = manifest::resolve_patch_path(rule);
                let exists = repo_root.join(&patch_path).exists();
                let marker = if exists { "exists" } else { "not found" };
                format!("{} -> {} ({marker})", rule.strategy, patch_path)
            } else {
                rule.strategy.to_string()
            };
            writeln!(
                w,
                "{}  rule[{i}] {}: {detail}",
                StatusTag::Ok.styled(),
                rule.path
            )?;
        }
    }

    // Stage 2 — reference validation
    let ref_ok = match manifest::validate_references(&m, repo_root) {
        Ok(()) => true,
        Err(e) => {
            writeln!(
                w,
                "{}  reference validation failed: {e}",
                StatusTag::Fail.styled()
            )?;
            false
        }
    };

    // Summary
    writeln!(w, "---")?;
    if schema_ok && ref_ok {
        writeln!(w, "{} files OK", m.files.len())?;
        Ok(ExitCode::SUCCESS)
    } else {
        let total = m.files.len();
        writeln!(w, "{total} files checked, errors found")?;
        Ok(ExitCode::FAILURE)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::panic)]

    use std::process::ExitCode;

    use tempfile::TempDir;

    use super::*;

    fn write_manifest(dir: &TempDir, content: &str) -> std::path::PathBuf {
        let path = dir.path().join("config.yml");
        std::fs::write(&path, content).unwrap();
        path
    }

    const VALID_MANIFEST: &str = r"
upstream:
  repo: owner/repo
  ref: main
files:
  - path: .github/ci.yml
    strategy: replace
";

    #[test]
    fn valid_manifest_returns_success() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let path = write_manifest(&dir, VALID_MANIFEST);
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let code = run(&path, dir.path(), &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS: {out}");
        assert!(out.contains("[OK"), "missing OK lines: {out}");
    }

    #[test]
    fn invalid_yaml_returns_failure() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let path = write_manifest(&dir, "not: valid: yaml: [[[");
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let code = run(&path, dir.path(), &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::FAILURE), "expected FAILURE: {out}");
    }

    #[test]
    fn missing_manifest_file_returns_failure() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.yml");
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let code = run(&path, dir.path(), &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::FAILURE), "expected FAILURE: {out}");
        assert!(out.contains("[FAIL"), "missing FAIL tag: {out}");
    }

    #[test]
    fn patch_rule_with_existing_file_shows_exists() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        // Create the expected patch file
        let patch_dir = dir.path().join(".github/gh-sync/patches");
        std::fs::create_dir_all(&patch_dir).unwrap();
        std::fs::write(patch_dir.join("Cargo.toml.patch"), b"--- a\n+++ b\n").unwrap();

        let manifest_yaml = r"
upstream:
  repo: owner/repo
files:
  - path: Cargo.toml
    strategy: patch
";
        let path = write_manifest(&dir, manifest_yaml);
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let code = run(&path, dir.path(), &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS: {out}");
        assert!(out.contains("exists"), "missing 'exists' marker: {out}");
    }

    #[test]
    fn patch_rule_with_missing_file_returns_failure() {
        // Arrange — patch file does NOT exist
        let dir = tempfile::tempdir().unwrap();
        let manifest_yaml = r"
upstream:
  repo: owner/repo
files:
  - path: Cargo.toml
    strategy: patch
";
        let path = write_manifest(&dir, manifest_yaml);
        let mut buf: Vec<u8> = Vec::new();

        // Act
        let code = run(&path, dir.path(), &mut buf).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(matches!(code, ExitCode::FAILURE), "expected FAILURE: {out}");
    }
}
