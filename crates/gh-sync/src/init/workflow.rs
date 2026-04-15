use std::path::Path;

/// Default output path for the generated workflow file.
pub const WORKFLOW_PATH: &str = ".github/workflows/gh-sync.yaml";

/// Workflow template with `{{version}}` and `{{upstream_manifest_line}}` placeholders.
const TEMPLATE: &str = "\
# yaml-language-server: $schema=https://json.schemastore.org/github-workflow.json
name: gh-sync check
on:
  push:
    branches: [main]
  pull_request:
    types: [opened, synchronize, reopened]
  schedule:
    - cron: \"0 18 * * *\" # daily at 03:00 JST
  workflow_dispatch:

permissions: {}

concurrency:
  group: gh-sync-${{ github.ref }}
  cancel-in-progress: ${{ startsWith(github.ref, 'refs/pull/') }}

jobs:
  gh-sync-check:
    name: gh-sync-check
    runs-on: ubuntu-latest
    timeout-minutes: 10
    permissions:
      contents: read
    steps:
      - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd # v6.0.2
        with:
          persist-credentials: false
      - uses: naa0yama/gh-sync@{{version}} # zizmor: ignore[unpinned-uses]
        with:
          token: ${{ secrets.GITHUB_TOKEN }}
{{upstream_manifest_line}}
";

/// Render the workflow template by substituting `{{version}}` and `{{upstream_manifest_line}}`.
///
/// - `version`: the gh-sync release tag (e.g. `v0.1.0`).
/// - `upstream_manifest`: when `Some`, emits an `upstream-manifest:` input line;
///   when `None`, emits a commented-out placeholder instead.
#[must_use]
pub fn render(version: &str, upstream_manifest: Option<&str>) -> String {
    let upstream_line = upstream_manifest.map_or_else(
        || {
            String::from(
                "          # upstream-manifest: owner/repo@main:.github/gh-sync/config.yaml",
            )
        },
        |v| format!("          upstream-manifest: {v}"),
    );
    TEMPLATE
        .replace("{{version}}", version)
        .replace("{{upstream_manifest_line}}", &upstream_line)
}

/// Write the rendered workflow file to `path`, creating parent directories as needed.
///
/// # Errors
///
/// Returns an error when the directory cannot be created or the file cannot be written.
pub fn write_workflow_file(
    path: &Path,
    version: &str,
    upstream_manifest: Option<&str>,
) -> anyhow::Result<()> {
    use anyhow::Context as _;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory '{}'", parent.display()))?;
    }
    std::fs::write(path, render(version, upstream_manifest))
        .with_context(|| format!("failed to write workflow file '{}'", path.display()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn render_substitutes_version() {
        let out = render("v1.2.3", None);
        assert!(out.contains("naa0yama/gh-sync@v1.2.3"));
        assert!(!out.contains("{{version}}"));
    }

    #[test]
    fn render_contains_required_fields() {
        let out = render("v0.1.0", None);
        assert!(out.contains("gh-sync check"));
        assert!(out.contains("actions/checkout@"));
        assert!(out.contains("secrets.GITHUB_TOKEN"));
        assert!(out.contains("contents: read"));
    }

    #[test]
    fn render_with_upstream_manifest_emits_active_line() {
        let out = render(
            "v0.1.0",
            Some("owner/repo@main:.github/gh-sync/config.yaml"),
        );
        assert!(
            out.contains("upstream-manifest: owner/repo@main:.github/gh-sync/config.yaml"),
            "missing upstream-manifest line: {out}"
        );
        assert!(
            !out.contains("{{upstream_manifest_line}}"),
            "placeholder not replaced: {out}"
        );
    }

    #[test]
    fn render_without_upstream_manifest_emits_comment() {
        let out = render("v0.1.0", None);
        assert!(
            out.contains("# upstream-manifest: owner/repo@main:.github/gh-sync/config.yaml"),
            "missing comment placeholder: {out}"
        );
        assert!(
            !out.contains("{{upstream_manifest_line}}"),
            "placeholder not replaced: {out}"
        );
    }

    #[test]
    fn write_workflow_file_creates_file_and_dirs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".github/workflows/gh-sync.yaml");
        write_workflow_file(&path, "v0.1.0", None).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("naa0yama/gh-sync@v0.1.0"));
    }

    #[test]
    fn write_workflow_file_overwrites_existing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("gh-sync.yaml");
        std::fs::write(&path, b"old content").unwrap();
        write_workflow_file(&path, "v0.2.0", None).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("v0.2.0"));
        assert!(!content.contains("old content"));
    }
}
