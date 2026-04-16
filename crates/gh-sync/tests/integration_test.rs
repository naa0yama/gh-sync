#![allow(clippy::unwrap_used)] // unwrap is permitted in test code
#![allow(missing_docs)] // doc comments are not required in test code

use assert_cmd::cargo_bin_cmd;
use predicates::prelude::{PredicateBooleanExt as _, predicate};

// ---------------------------------------------------------------------------
// Helpers for fake-gh integration tests
// ---------------------------------------------------------------------------

/// Absolute path to `tests/fixtures/fake-gh/` (contains the fake `gh` script).
fn fake_gh_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-gh")
}

/// Absolute path to the given scenario JSON file inside `tests/fixtures/scenarios/`.
fn scenario(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/scenarios")
        .join(name)
}

/// Build a `PATH` string that puts `fake_gh_dir()` first, so that
/// `Command::new("gh")` inside `gh-sync` finds the fake script.
fn path_with_fake_gh() -> String {
    let original = std::env::var("PATH").unwrap_or_default();
    format!("{}:{original}", fake_gh_dir().display())
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_help() {
    let mut cmd = cargo_bin_cmd!("gh-sync");
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("sync"))
        .stdout(predicate::str::contains("init"));
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_sync_help() {
    let mut cmd = cargo_bin_cmd!("gh-sync");
    cmd.args(["sync", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("file"))
        .stdout(predicate::str::contains("repo"));
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_sync_file_help() {
    let mut cmd = cargo_bin_cmd!("gh-sync");
    cmd.args(["sync", "file", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("manifest"))
        .stdout(predicate::str::contains("dry-run"));
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_sync_repo_help() {
    let mut cmd = cargo_bin_cmd!("gh-sync");
    cmd.args(["sync", "repo", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("manifest"))
        .stdout(predicate::str::contains("ci-check"));
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_init_help() {
    let mut cmd = cargo_bin_cmd!("gh-sync");
    cmd.args(["init", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("repo"))
        .stdout(predicate::str::contains("output"));
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_version_flag() {
    let mut cmd = cargo_bin_cmd!("gh-sync");
    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("gh-sync"))
        .stdout(predicate::str::contains("(rev:"));
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_version_short_flag() {
    let mut cmd = cargo_bin_cmd!("gh-sync");
    cmd.arg("-V")
        .assert()
        .success()
        .stdout(predicate::str::contains("gh-sync"))
        .stdout(predicate::str::contains("(rev:"));
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_pr_help() {
    let mut cmd = cargo_bin_cmd!("gh-sync");
    cmd.args(["pr", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("title"))
        .stdout(predicate::str::contains("branch"));
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_validate_and_ci_check_conflict() {
    // --validate and --ci-check are mutually exclusive (sync file only)
    let mut cmd = cargo_bin_cmd!("gh-sync");
    cmd.args(["sync", "file", "--validate", "--ci-check"])
        .assert()
        .failure();
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_validate_and_patch_refresh_conflict() {
    // --validate and --patch-refresh are mutually exclusive
    let mut cmd = cargo_bin_cmd!("gh-sync");
    cmd.args(["sync", "file", "--validate", "--patch-refresh"])
        .assert()
        .failure();
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_ci_check_and_patch_refresh_conflict() {
    // --ci-check and --patch-refresh are mutually exclusive
    let mut cmd = cargo_bin_cmd!("gh-sync");
    cmd.args(["sync", "file", "--ci-check", "--patch-refresh"])
        .assert()
        .failure();
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_from_upstream_and_select_conflict() {
    // init --from-upstream and --select are mutually exclusive
    let mut cmd = cargo_bin_cmd!("gh-sync");
    cmd.args(["init", "--from-upstream", "--select"])
        .assert()
        .failure();
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_init_with_workflow_in_help() {
    // --with-workflow flag must appear in init --help
    let mut cmd = cargo_bin_cmd!("gh-sync");
    cmd.args(["init", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("with-workflow"));
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_validate_missing_manifest() {
    // Validation with a non-existent manifest returns failure
    let mut cmd = cargo_bin_cmd!("gh-sync");
    cmd.args([
        "sync",
        "file",
        "--validate",
        "--manifest",
        "/nonexistent/config.yaml",
    ])
    .assert()
    .failure();
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_dry_run_missing_manifest() {
    // Default sync file (dry-run) with a non-existent manifest returns failure
    let mut cmd = cargo_bin_cmd!("gh-sync");
    cmd.args([
        "sync",
        "file",
        "--dry-run",
        "--manifest",
        "/nonexistent/config.yaml",
    ])
    .assert()
    .failure();
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_sync_repo_dry_run_missing_manifest() {
    // sync repo with a non-existent manifest returns failure
    let mut cmd = cargo_bin_cmd!("gh-sync");
    cmd.args([
        "sync",
        "repo",
        "--dry-run",
        "--manifest",
        "/nonexistent/config.yaml",
    ])
    .assert()
    .failure();
}

// ---------------------------------------------------------------------------
// Fake-gh integration tests
//
// These tests replace the real `gh` CLI with a Python script that reads a
// scenario JSON file and returns canned responses.  They exercise the full
// `gh-sync sync repo` pipeline — manifest loading → API call construction →
// JSON parsing → change detection → preview output — while documenting the
// GitHub API response format that was working when the fixture was recorded.
//
// If `gh` CLI changes its response shape, these tests will fail, giving early
// warning of API compatibility regressions.
// ---------------------------------------------------------------------------

/// Minimal manifest YAML for sync-repo tests (only `description` in spec so
/// that exactly two `gh` calls are made: `detect_repo` + `get_repo`).
/// The `files` section uses `replace` strategy so no patch file needs to exist.
const MANIFEST_DESCRIPTION_DRIFT: &str = "\
upstream:
  repo: upstream/repo
files:
  - path: .github/CODEOWNERS
    strategy: replace
spec:
  description: \"new description\"
";

const MANIFEST_DESCRIPTION_MATCH: &str = "\
upstream:
  repo: upstream/repo
files:
  - path: .github/CODEOWNERS
    strategy: replace
spec:
  description: \"current description\"
";

#[test]
#[cfg_attr(miri, ignore)]
fn test_fake_gh_sync_repo_dry_run_shows_description_change() {
    // Arrange — write manifest to temp dir
    let dir = tempfile::TempDir::new().unwrap();
    let manifest = dir.path().join("config.yaml");
    std::fs::write(&manifest, MANIFEST_DESCRIPTION_DRIFT).unwrap();

    // Act — run `gh-sync sync repo --dry-run` with the fake gh script
    let mut cmd = cargo_bin_cmd!("gh-sync");
    cmd.args(["sync", "repo", "--dry-run", "--manifest"])
        .arg(&manifest)
        .env("PATH", path_with_fake_gh())
        .env(
            "GH_FAKE_SCENARIO",
            scenario("sync_repo_description_drift.json"),
        );

    // Assert — exits 0 (dry-run never fails on drift), output shows CHANGED
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("description"))
        .stdout(predicate::str::contains("CHANGED").or(predicate::str::contains("changed")));
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_fake_gh_sync_repo_ci_check_fails_on_description_drift() {
    // Arrange
    let dir = tempfile::TempDir::new().unwrap();
    let manifest = dir.path().join("config.yaml");
    std::fs::write(&manifest, MANIFEST_DESCRIPTION_DRIFT).unwrap();

    // Act — --ci-check exits 1 when changes are detected
    let mut cmd = cargo_bin_cmd!("gh-sync");
    cmd.args(["sync", "repo", "--ci-check", "--manifest"])
        .arg(&manifest)
        .env("PATH", path_with_fake_gh())
        .env(
            "GH_FAKE_SCENARIO",
            scenario("sync_repo_description_drift.json"),
        );

    // Assert
    cmd.assert()
        .failure()
        .stdout(predicate::str::contains("description"));
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_fake_gh_sync_repo_dry_run_no_drift_succeeds() {
    // Arrange — spec matches live state (description = "current description")
    let dir = tempfile::TempDir::new().unwrap();
    let manifest = dir.path().join("config.yaml");
    std::fs::write(&manifest, MANIFEST_DESCRIPTION_MATCH).unwrap();

    // Act
    let mut cmd = cargo_bin_cmd!("gh-sync");
    cmd.args(["sync", "repo", "--dry-run", "--manifest"])
        .arg(&manifest)
        .env("PATH", path_with_fake_gh())
        .env("GH_FAKE_SCENARIO", scenario("sync_repo_no_drift.json"));

    // Assert — exits 0, output indicates no changes
    cmd.assert().success().stdout(
        predicate::str::contains("up to date")
            .or(predicate::str::contains("OK"))
            .or(predicate::str::contains("nothing")),
    );
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_fake_gh_sync_repo_detect_repo_failure_exits_nonzero() {
    // Arrange — fake gh returns exit 1 for detect_repo
    let dir = tempfile::TempDir::new().unwrap();
    let manifest = dir.path().join("config.yaml");
    std::fs::write(&manifest, MANIFEST_DESCRIPTION_DRIFT).unwrap();

    // Act
    let mut cmd = cargo_bin_cmd!("gh-sync");
    cmd.args(["sync", "repo", "--dry-run", "--manifest"])
        .arg(&manifest)
        .env("PATH", path_with_fake_gh())
        .env("GH_FAKE_SCENARIO", scenario("detect_repo_failure.json"));

    // Assert — must fail because detect_repo failed
    cmd.assert().failure();
}
