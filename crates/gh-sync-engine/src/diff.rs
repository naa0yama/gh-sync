/// Generate a unified diff between `old` and `new` bytes.
///
/// The `path` string is used as the label in the `---`/`+++` header lines.
/// Returns an empty string when the two byte slices are identical.
///
/// # Errors
///
/// Returns an error when the temp directory or files cannot be created, or
/// when the `diff` binary cannot be spawned or exits with code 2 (error).
#[allow(clippy::module_name_repetitions)] // "unified_diff" in module "diff" is intentional
pub fn unified_diff(path: &str, old: &[u8], new: &[u8]) -> anyhow::Result<String> {
    use anyhow::Context as _;

    if old == new {
        return Ok(String::new());
    }

    let dir = tempfile::tempdir().context("failed to create temp dir for diff")?;
    let old_path = dir.path().join("old");
    let new_path = dir.path().join("new");

    std::fs::write(&old_path, old).context("failed to write old content for diff")?;
    std::fs::write(&new_path, new).context("failed to write new content for diff")?;

    let output = std::process::Command::new("diff")
        .args([
            "-u",
            &format!("--label=a/{path}"),
            &format!("--label=b/{path}"),
            old_path.to_str().context("non-UTF-8 old path")?,
            new_path.to_str().context("non-UTF-8 new path")?,
        ])
        .output()
        .context("failed to spawn `diff`")?;

    // diff exit codes: 0 = identical, 1 = files differ (normal), 2 = error
    match output.status.code() {
        Some(0 | 1) => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
        // NOTEST(io): `diff` exit code 2 indicates an invocation or I/O error —
        // not triggerable with valid temp file paths under normal conditions
        _ => {
            anyhow::bail!(
                "`diff` exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )
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

    #[cfg_attr(miri, ignore)]
    #[test]
    fn identical_content_returns_empty_string() {
        // Arrange
        let content = b"hello\nworld\n";

        // Act
        let diff = unified_diff("file.txt", content, content).unwrap();

        // Assert
        assert!(diff.is_empty(), "expected empty diff for identical content");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn differing_content_returns_unified_diff() {
        // Arrange
        let old = b"hello\n";
        let new = b"world\n";

        // Act
        let diff = unified_diff("file.txt", old, new).unwrap();

        // Assert
        assert!(
            diff.contains("--- a/file.txt"),
            "missing old label in diff header"
        );
        assert!(
            diff.contains("+++ b/file.txt"),
            "missing new label in diff header"
        );
        assert!(diff.contains("-hello"), "missing removed line in diff");
        assert!(diff.contains("+world"), "missing added line in diff");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn path_label_appears_in_header() {
        // Arrange
        let path = ".github/workflows/ci.yml";

        // Act
        let diff = unified_diff(path, b"old\n", b"new\n").unwrap();

        // Assert
        assert!(
            diff.contains(&format!("--- a/{path}")),
            "expected 'a/' prefixed path in diff header"
        );
        assert!(
            diff.contains(&format!("+++ b/{path}")),
            "expected 'b/' prefixed path in diff header"
        );
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn empty_old_and_new_returns_empty_string() {
        // Arrange / Act
        let diff = unified_diff("empty.txt", b"", b"").unwrap();

        // Assert
        assert!(diff.is_empty(), "expected empty diff for two empty inputs");
    }
}
