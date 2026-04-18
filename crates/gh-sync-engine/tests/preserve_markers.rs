#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]
#![allow(missing_docs)]

use std::process::ExitCode;

use gh_sync_engine::mode::patch_refresh;
use gh_sync_engine::upstream::testing::MockFetcher;
use gh_sync_manifest::{Manifest, Rule, Strategy, Upstream};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn make_manifest(path: &str, preserve_markers: Option<bool>) -> Manifest {
    Manifest {
        upstream: Upstream {
            repo: "owner/repo".to_owned(),
            ref_: "main".to_owned(),
        },
        spec: None,
        files: vec![Rule {
            path: path.to_owned(),
            strategy: Strategy::Patch,
            source: None,
            patch: None,
            preserve_markers,
        }],
    }
}

// ---------------------------------------------------------------------------
// Case 1 — local-only marker block is excluded from the generated patch
// ---------------------------------------------------------------------------

/// When `preserve_markers` is true, content inside marker blocks is stripped
/// from the local file before diffing. If the only difference from upstream
/// is inside a marker block, the resulting patch file must be empty.
#[cfg_attr(miri, ignore)]
#[test]
fn marker_local_only_block_excluded_from_patch() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let upstream_bytes = b"[workspace.dependencies]\nanyhow = \"1.0\"\n";
    // local = upstream content + a marker block; the block is local-only
    let local_bytes = b"[workspace.dependencies]\nanyhow = \"1.0\"\n# gh-sync:keep-start\ngh-sync-engine = { version = \"0.2.1\" }\n# gh-sync:keep-end\n";
    std::fs::write(dir.path().join("Cargo.toml"), local_bytes).unwrap();
    let manifest = make_manifest("Cargo.toml", Some(true));
    let fetcher = MockFetcher::content(upstream_bytes.to_vec());
    let mut buf: Vec<u8> = Vec::new();

    // Act
    let code = patch_refresh::run(&manifest, dir.path(), &fetcher, &mut buf).unwrap();

    // Assert
    let out = String::from_utf8(buf).unwrap();
    assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS: {out}");
    let patch_path = dir.path().join(".github/gh-sync/patches/Cargo.toml.patch");
    assert!(patch_path.exists(), "patch file should be created");
    assert_eq!(
        std::fs::read(&patch_path).unwrap(),
        b"",
        "patch file should be empty: marker content excluded from diff"
    );
}

// ---------------------------------------------------------------------------
// Case 2 — version drift inside a marker block is excluded from the patch
// ---------------------------------------------------------------------------

/// When the local file has the upstream content outside the marker and a
/// different version *inside* the marker, stripping the marker leaves local
/// equal to upstream — so the patch is empty.
///
/// Note: the byte sequence used here (`version` appearing twice) is a
/// synthetic, byte-level scenario that is not valid TOML (duplicate key).
/// It tests the strip logic in isolation; real-world usage places the
/// *entire* downstream-only block inside the marker, not a replacement of an
/// upstream line.
#[cfg_attr(miri, ignore)]
#[test]
fn marker_version_drift_excluded_from_patch() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let upstream_bytes = b"version = \"0.1.0\"\n";
    // local = upstream line (outside marker) + drifted line inside marker
    // (synthetic byte-level test; not representable as valid TOML)
    let local_bytes =
        b"version = \"0.1.0\"\n# gh-sync:keep-start\nversion = \"0.2.1\"\n# gh-sync:keep-end\n";
    std::fs::write(dir.path().join("Cargo.toml"), local_bytes).unwrap();
    let manifest = make_manifest("Cargo.toml", Some(true));
    let fetcher = MockFetcher::content(upstream_bytes.to_vec());
    let mut buf: Vec<u8> = Vec::new();

    // Act
    let code = patch_refresh::run(&manifest, dir.path(), &fetcher, &mut buf).unwrap();

    // Assert
    let out = String::from_utf8(buf).unwrap();
    assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS: {out}");
    let patch_path = dir.path().join(".github/gh-sync/patches/Cargo.toml.patch");
    assert!(patch_path.exists(), "patch file should be created");
    assert_eq!(
        std::fs::read(&patch_path).unwrap(),
        b"",
        "patch file should be empty: version drift inside marker excluded"
    );
}

// ---------------------------------------------------------------------------
// Case 3 — drift outside a marker block IS included in the patch
// ---------------------------------------------------------------------------

/// Changes outside marker blocks are not stripped and must appear in the
/// generated patch. Here `anyhow` changes from 1.0 to 2.0 outside the
/// marker, so the patch must be non-empty.
#[cfg_attr(miri, ignore)]
#[test]
fn marker_external_drift_included_in_patch() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let upstream_bytes =
        b"# gh-sync:keep-start\nversion = \"0.1.0\"\n# gh-sync:keep-end\nanyhow = \"1.0\"\n";
    let local_bytes =
        b"# gh-sync:keep-start\nversion = \"0.2.1\"\n# gh-sync:keep-end\nanyhow = \"2.0\"\n";
    std::fs::write(dir.path().join("Cargo.toml"), local_bytes).unwrap();
    let manifest = make_manifest("Cargo.toml", Some(true));
    let fetcher = MockFetcher::content(upstream_bytes.to_vec());
    let mut buf: Vec<u8> = Vec::new();

    // Act
    let code = patch_refresh::run(&manifest, dir.path(), &fetcher, &mut buf).unwrap();

    // Assert
    let out = String::from_utf8(buf).unwrap();
    assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS: {out}");
    let patch_path = dir.path().join(".github/gh-sync/patches/Cargo.toml.patch");
    assert!(patch_path.exists(), "patch file should be created");
    assert!(
        !std::fs::read(&patch_path).unwrap().is_empty(),
        "patch file must be non-empty: anyhow drift outside marker must appear in diff"
    );
}

// ---------------------------------------------------------------------------
// Case 4 — orphan keep-start produces FAILURE with [FAIL in output
// ---------------------------------------------------------------------------

/// An unclosed `keep-start` marker causes `strip_marker_blocks` to return
/// `MarkerError::Unbalanced`. The run must report failure.
#[cfg_attr(miri, ignore)]
#[test]
fn orphan_start_marker_fails_with_unbalanced() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let local_bytes = b"anyhow = \"1.0\"\n# gh-sync:keep-start\nversion = \"0.2.1\"\n";
    std::fs::write(dir.path().join("Cargo.toml"), local_bytes).unwrap();
    let manifest = make_manifest("Cargo.toml", Some(true));
    let fetcher = MockFetcher::content(b"anyhow = \"1.0\"\n".to_vec());
    let mut buf: Vec<u8> = Vec::new();

    // Act
    let code = patch_refresh::run(&manifest, dir.path(), &fetcher, &mut buf).unwrap();

    // Assert
    let out = String::from_utf8(buf).unwrap();
    assert!(matches!(code, ExitCode::FAILURE), "expected FAILURE: {out}");
    assert!(out.contains("[FAIL"), "expected [FAIL in output: {out}");
}

// ---------------------------------------------------------------------------
// Case 5 — nested keep-start produces FAILURE with [FAIL in output
// ---------------------------------------------------------------------------

/// A `keep-start` encountered inside an already-open block returns
/// `MarkerError::Nested`. The run must report failure.
#[cfg_attr(miri, ignore)]
#[test]
fn nested_start_marker_fails_with_nested() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let local_bytes = b"# gh-sync:keep-start\n# gh-sync:keep-start\ninner\n# gh-sync:keep-end\n# gh-sync:keep-end\n";
    std::fs::write(dir.path().join("Cargo.toml"), local_bytes).unwrap();
    let manifest = make_manifest("Cargo.toml", Some(true));
    let fetcher = MockFetcher::content(b"anyhow = \"1.0\"\n".to_vec());
    let mut buf: Vec<u8> = Vec::new();

    // Act
    let code = patch_refresh::run(&manifest, dir.path(), &fetcher, &mut buf).unwrap();

    // Assert
    let out = String::from_utf8(buf).unwrap();
    assert!(matches!(code, ExitCode::FAILURE), "expected FAILURE: {out}");
    assert!(out.contains("[FAIL"), "expected [FAIL in output: {out}");
}

// ---------------------------------------------------------------------------
// Case 6 — preserve_markers disabled: marker lines treated as regular content
// ---------------------------------------------------------------------------

/// When `preserve_markers` is `None` (disabled), marker comment lines are
/// treated as ordinary content. If the local file has marker lines that the
/// upstream does not, those lines appear in the patch as additions.
#[cfg_attr(miri, ignore)]
#[test]
fn preserve_markers_false_includes_marker_lines_in_patch() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let upstream_bytes = b"anyhow = \"1.0\"\n";
    let local_bytes = b"# gh-sync:keep-start\nanyhow = \"1.0\"\n# gh-sync:keep-end\n";
    std::fs::write(dir.path().join("Cargo.toml"), local_bytes).unwrap();
    let manifest = make_manifest("Cargo.toml", None);
    let fetcher = MockFetcher::content(upstream_bytes.to_vec());
    let mut buf: Vec<u8> = Vec::new();

    // Act
    let code = patch_refresh::run(&manifest, dir.path(), &fetcher, &mut buf).unwrap();

    // Assert
    let out = String::from_utf8(buf).unwrap();
    assert!(matches!(code, ExitCode::SUCCESS), "expected SUCCESS: {out}");
    let patch_path = dir.path().join(".github/gh-sync/patches/Cargo.toml.patch");
    assert!(patch_path.exists(), "patch file should be created");
    assert!(
        !std::fs::read(&patch_path).unwrap().is_empty(),
        "patch file must be non-empty: marker lines treated as regular content"
    );
}
