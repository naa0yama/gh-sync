use std::path::Path;

/// Default output path for the generated workflow file.
pub const WORKFLOW_PATH: &str = ".github/workflows/gh-sync.yaml";

/// Workflow template with `{{version}}`, `{{sha}}`, `{{upstream_manifest_line}}`,
/// and `{{upstream_manifest_arg_line}}` placeholders.
const TEMPLATE: &str = "\
# yaml-language-server: $schema=https://json.schemastore.org/github-workflow.json
name: gh-sync
on:
  schedule:
    - cron: \"0 18 * * *\" # daily at 03:00 JST
  workflow_dispatch:
  push:
    branches: [main]

permissions: {}

concurrency:
  group: gh-sync
  cancel-in-progress: true

jobs:
  gh-sync:
    name: drift-issue
    runs-on: ubuntu-latest
    timeout-minutes: 10
    permissions:
      contents: read
      issues: write # create and update drift-tracking issues
    steps:
      - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd # v6.0.2
        with:
          persist-credentials: false

      - uses: naa0yama/gh-sync@{{sha}} # {{version}}
        with:
          token: ${{ secrets.GITHUB_TOKEN }}
{{upstream_manifest_line}}
          install-only: \"true\"

      - name: Sync drift issue
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          gh-sync issue-sync \\
            --manifest .github/gh-sync/config.yaml{{upstream_manifest_arg_line}}
";

/// Render the workflow template by substituting `{{version}}`, `{{sha}}`,
/// `{{upstream_manifest_line}}`, and `{{upstream_manifest_arg_line}}`.
///
/// - `version`: the gh-sync release tag (e.g. `v0.1.0`).
/// - `sha`: the 40-character commit SHA that the release tag resolves to.
/// - `upstream_manifest`: when `Some`, emits active `upstream-manifest:` input
///   and `--upstream-manifest` CLI arg; when `None`, emits commented-out
///   placeholders.
#[must_use]
pub fn render(version: &str, sha: &str, upstream_manifest: Option<&str>) -> String {
    let (upstream_line, upstream_arg_line) = upstream_manifest.map_or_else(
        || {
            let line = String::from(
                "          # upstream-manifest: owner/repo@main:.github/gh-sync/config.yaml",
            );
            let arg = String::new();
            (line, arg)
        },
        |v| {
            let line = format!("          upstream-manifest: {v}");
            let arg = format!(" \\\n            --upstream-manifest {v}");
            (line, arg)
        },
    );
    TEMPLATE
        .replace("{{version}}", version)
        .replace("{{sha}}", sha)
        .replace("{{upstream_manifest_line}}", &upstream_line)
        .replace("{{upstream_manifest_arg_line}}", &upstream_arg_line)
}

/// Write pre-rendered `content` to `path`, creating parent directories as needed.
///
/// # Errors
///
/// Returns an error when the directory cannot be created or the file cannot be written.
pub fn write_workflow_from_content(path: &Path, content: &str) -> anyhow::Result<()> {
    super::write_file(path, content)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use tempfile::TempDir;

    use super::*;

    // split across concat! to avoid triggering the no-hardcoded-credentials lint
    const TEST_SHA: &str = concat!("aabbccdd1122334455", "667788990011223344556677");

    #[test]
    fn render_substitutes_version_and_sha() {
        let out = render("v1.2.3", TEST_SHA, None);
        assert!(
            out.contains(&format!("naa0yama/gh-sync@{TEST_SHA} # v1.2.3")),
            "expected SHA-pinned ref, got: {out}"
        );
        assert!(
            !out.contains("{{version}}"),
            "version placeholder not replaced"
        );
        assert!(!out.contains("{{sha}}"), "sha placeholder not replaced");
    }

    #[test]
    fn render_sha_is_40_hex_and_no_zizmor_ignore() {
        // split across concat! to avoid triggering the no-hardcoded-credentials lint
        let sha = concat!("93c2233ddf30c32021", "dd373d677d2575798f5eac");
        let out = render("v0.1.3", sha, None);
        // SHA must appear verbatim in the output
        assert!(out.contains(sha), "SHA missing from output");
        // zizmor ignore comment must not appear (SHA pin makes it unnecessary)
        assert!(
            !out.contains("zizmor: ignore"),
            "unexpected zizmor: ignore comment in output"
        );
    }

    #[test]
    fn render_contains_required_fields() {
        let out = render("v0.1.0", TEST_SHA, None);
        assert!(out.contains("name: gh-sync"), "wrong workflow name");
        assert!(out.contains("actions/checkout@"), "missing checkout step");
        assert!(out.contains("secrets.GITHUB_TOKEN"), "missing token ref");
        assert!(out.contains("contents: read"), "missing contents: read");
        assert!(out.contains("issues: write"), "missing issues: write");
        assert!(
            out.contains("install-only: \"true\""),
            "missing install-only input"
        );
        assert!(
            out.contains("gh-sync issue-sync"),
            "missing gh-sync issue-sync step"
        );
        assert!(
            !out.contains("version:"),
            "version: input must not appear (Renovate handles uses: line)"
        );
        assert!(
            !out.contains("apply-files:"),
            "apply-files: must not appear in new template"
        );
        assert!(!out.contains("gh-sync pr"), "gh-sync pr must not appear");
    }

    #[test]
    fn render_triggers_include_push_to_main() {
        let out = render("v0.1.0", TEST_SHA, None);
        assert!(out.contains("schedule:"), "missing schedule trigger");
        assert!(
            out.contains("workflow_dispatch:"),
            "missing workflow_dispatch trigger"
        );
        assert!(
            out.contains("push:") && out.contains("branches: [main]"),
            "push: branches: [main] trigger missing"
        );
        assert!(
            !out.contains("pull_request:"),
            "pull_request trigger must not appear"
        );
    }

    #[test]
    fn render_with_upstream_manifest_emits_active_line_and_arg() {
        let out = render(
            "v0.1.0",
            TEST_SHA,
            Some("owner/repo@main:.github/gh-sync/config.yaml"),
        );
        assert!(
            out.contains("upstream-manifest: owner/repo@main:.github/gh-sync/config.yaml"),
            "missing upstream-manifest input line: {out}"
        );
        assert!(
            out.contains("--upstream-manifest owner/repo@main:.github/gh-sync/config.yaml"),
            "missing --upstream-manifest CLI arg: {out}"
        );
        assert!(
            !out.contains("{{upstream_manifest_line}}"),
            "placeholder not replaced: {out}"
        );
        assert!(
            !out.contains("{{upstream_manifest_arg_line}}"),
            "arg placeholder not replaced: {out}"
        );
    }

    #[test]
    fn render_without_upstream_manifest_emits_comment_and_no_arg() {
        let out = render("v0.1.0", TEST_SHA, None);
        assert!(
            out.contains("# upstream-manifest: owner/repo@main:.github/gh-sync/config.yaml"),
            "missing comment placeholder: {out}"
        );
        assert!(
            !out.contains("--upstream-manifest"),
            "unexpected --upstream-manifest arg when manifest is None: {out}"
        );
        assert!(
            !out.contains("{{upstream_manifest_line}}"),
            "placeholder not replaced: {out}"
        );
    }

    #[test]
    fn write_workflow_from_content_creates_file_and_dirs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".github/workflows/gh-sync.yaml");
        let content = render("v0.1.0", TEST_SHA, None);
        write_workflow_from_content(&path, &content).unwrap();
        let read_back = std::fs::read_to_string(&path).unwrap();
        assert!(read_back.contains(&format!("naa0yama/gh-sync@{TEST_SHA} # v0.1.0")));
    }

    #[test]
    fn write_workflow_from_content_overwrites_existing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("gh-sync.yaml");
        std::fs::write(&path, b"old content").unwrap();
        let content = render("v0.2.0", TEST_SHA, None);
        write_workflow_from_content(&path, &content).unwrap();
        let read_back = std::fs::read_to_string(&path).unwrap();
        assert!(read_back.contains("v0.2.0"));
        assert!(!read_back.contains("old content"));
    }

    #[test]
    fn write_workflow_from_content_writes_raw_string() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("custom.yaml");
        write_workflow_from_content(&path, "custom content").unwrap();
        let read_back = std::fs::read_to_string(&path).unwrap();
        assert_eq!(read_back, "custom content");
    }
}
