use std::fmt::Write as _;
use std::io::{self, IsTerminal as _};

use anyhow::Context as _;
use serde::Serialize;

use crate::sync::manifest::{Manifest, Rule, Upstream};
use crate::sync::upstream::UpstreamFetcher;

/// Build a [`Manifest`] from a list of entries selected by the user.
///
/// This function is pure (no I/O) and is tested independently from the
/// interactive prompt layer.
#[must_use]
pub fn build_manifest(repo: &str, ref_: &str, files: Vec<Rule>) -> Manifest {
    Manifest {
        upstream: Upstream {
            repo: repo.to_owned(),
            ref_: ref_.to_owned(),
        },
        spec: None,
        files,
    }
}

/// Serialize a manifest to YAML with the schema comment prepended.
///
/// Rules are sorted alphabetically by path and grouped under YAML comment
/// headers that reflect the parent directory of each group.
///
/// # Errors
///
/// Returns an error when YAML serialization fails.
pub fn manifest_to_yaml(manifest: &Manifest) -> anyhow::Result<String> {
    // Serialize only the `upstream:` section so we can append a hand-built
    // `spec.files:` block that includes directory-group comment headers.
    #[derive(Serialize)]
    struct UpstreamOnly<'a> {
        upstream: &'a Upstream,
    }
    let upstream_yaml = serde_yml::to_string(&UpstreamOnly {
        upstream: &manifest.upstream,
    })
    .context("failed to serialize upstream to YAML")?;

    // Sort files: directory files first (sorted by path), root-level files last.
    let mut sorted: Vec<&Rule> = manifest.files.iter().collect();
    sorted.sort_by(|a, b| {
        let a_in_dir = a.path.contains('/');
        let b_in_dir = b.path.contains('/');
        match (a_in_dir, b_in_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.path.cmp(&b.path),
        }
    });

    let files_yaml = build_files_yaml(&sorted);
    Ok(format!(
        "{}{upstream_yaml}{files_yaml}",
        super::schema::comment()
    ))
}

/// Return the parent-directory portion of `path`, or `None` for root-level files.
fn parent_dir(path: &str) -> Option<&str> {
    let i = path.rfind('/')?;
    path.get(..i)
}

/// Build the `files:` YAML block with directory-group comment headers.
///
/// Files must already be sorted by path before calling.
/// - Directory groups get a `# {dir}` comment header.
/// - Root-level files (no parent directory) are always last, under `# others`.
fn build_files_yaml(files: &[&Rule]) -> String {
    let mut out = String::from("files:\n");
    // `None`     — no group emitted yet (initial sentinel)
    // `Some(d)`  — currently inside group `d` (where `d` itself is `Option<&str>`)
    let mut current_group: Option<Option<&str>> = None;

    for rule in files {
        let dir = parent_dir(&rule.path);

        // Emit a header when the group changes (or on the very first entry).
        let group_changed = current_group != Some(dir);
        if group_changed {
            if current_group.is_some() {
                out.push('\n'); // blank line between groups
            }
            match dir {
                Some(d) => {
                    let _ = writeln!(out, "  # {d}");
                }
                None => {
                    let _ = writeln!(out, "  # others");
                }
            }
            current_group = Some(dir);
        }

        let path = &rule.path;
        let _ = writeln!(out, "  - path: {path}");
        let _ = writeln!(out, "    strategy: {}", rule.strategy);
        if let Some(source) = &rule.source {
            let _ = writeln!(out, "    source: {source}");
        }
        if let Some(patch) = &rule.patch {
            let _ = writeln!(out, "    patch: {patch}");
        }
    }

    out
}

/// Run the interactive file-selection wizard.
///
/// Recursively lists all files from the upstream repository using the Git Trees
/// API and prompts the user to:
/// 1. Select which files to include
/// 2. Choose a strategy for each selected file
///
/// Returns the generated YAML config content.
///
/// # Errors
///
/// Returns an error when:
/// - stdin is not a TTY
/// - listing files from upstream fails
/// - the user cancels the prompt (Ctrl-C / Esc)
/// - YAML serialization fails
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn run_interactive(
    fetcher: &dyn UpstreamFetcher,
    repo: &str,
    ref_: &str,
    _dir_path: &str,
) -> anyhow::Result<String> {
    if !io::stdin().is_terminal() {
        anyhow::bail!(
            "`--select` requires an interactive terminal\n\
             hint: use `--from-upstream` for non-interactive mode"
        );
    }

    let entries = fetcher
        .list_all_files(repo, ref_)
        .with_context(|| format!("failed to list files of '{repo}' at '{ref_}'"))?;

    if entries.is_empty() {
        anyhow::bail!("no files found in '{repo}' at ref '{ref_}'");
    }

    let paths: Vec<String> = entries.into_iter().map(|e| e.path).collect();
    let selected = super::select::pick(&paths)?;

    let rules: Vec<Rule> = selected
        .into_iter()
        .map(|f| Rule {
            path: f.path,
            strategy: f.strategy,
            source: None,
            patch: None,
        })
        .collect();

    let manifest = build_manifest(repo, ref_, rules);
    manifest_to_yaml(&manifest)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use crate::sync::manifest::Strategy;

    use super::*;

    #[test]
    fn build_manifest_sets_fields() {
        let files = vec![Rule {
            path: String::from("foo.txt"),
            strategy: Strategy::Replace,
            source: None,
            patch: None,
        }];
        let m = build_manifest("owner/repo", "main", files);
        assert_eq!(m.upstream.repo, "owner/repo");
        assert_eq!(m.upstream.ref_, "main");
        assert_eq!(m.files.len(), 1);
        assert_eq!(m.files.first().map(|r| r.path.as_str()), Some("foo.txt"));
    }

    #[test]
    fn manifest_to_yaml_prepends_schema_comment() {
        let files = vec![Rule {
            path: String::from("foo.txt"),
            strategy: Strategy::Replace,
            source: None,
            patch: None,
        }];
        let m = build_manifest("owner/repo", "main", files);
        let yaml = manifest_to_yaml(&m).unwrap();
        assert!(
            yaml.starts_with("# yaml-language-server"),
            "missing schema comment"
        );
        assert!(yaml.contains("upstream:"), "missing upstream section");
        assert!(yaml.contains("files:"), "missing files section");
        assert!(yaml.contains("foo.txt"), "missing file path");
    }

    #[test]
    fn manifest_to_yaml_sorts_rules_by_path() {
        let rules = vec![
            Rule {
                path: String::from(".github/workflows/release.yaml"),
                strategy: Strategy::Replace,
                source: None,
                patch: None,
            },
            Rule {
                path: String::from(".github/workflows/ci.yaml"),
                strategy: Strategy::Replace,
                source: None,
                patch: None,
            },
        ];
        let m = build_manifest("owner/repo", "main", rules);
        let yaml = manifest_to_yaml(&m).unwrap();
        let ci_pos = yaml.find("ci.yaml").unwrap();
        let release_pos = yaml.find("release.yaml").unwrap();
        assert!(
            ci_pos < release_pos,
            "ci.yaml should appear before release.yaml"
        );
    }

    #[test]
    fn manifest_to_yaml_adds_directory_group_headers() {
        let rules = vec![
            Rule {
                path: String::from(".github/workflows/ci.yaml"),
                strategy: Strategy::Replace,
                source: None,
                patch: None,
            },
            Rule {
                path: String::from("docs/spec.md"),
                strategy: Strategy::CreateOnly,
                source: None,
                patch: None,
            },
        ];
        let m = build_manifest("owner/repo", "main", rules);
        let yaml = manifest_to_yaml(&m).unwrap();
        assert!(
            yaml.contains("# .github/workflows"),
            "missing .github/workflows group header"
        );
        assert!(yaml.contains("# docs"), "missing docs group header");
    }

    #[test]
    fn manifest_to_yaml_root_files_grouped_as_others() {
        let rules = vec![Rule {
            path: String::from("README.md"),
            strategy: Strategy::CreateOnly,
            source: None,
            patch: None,
        }];
        let m = build_manifest("owner/repo", "main", rules);
        let yaml = manifest_to_yaml(&m).unwrap();
        assert!(
            yaml.contains("# others"),
            "root-level file should appear under # others"
        );
    }

    #[test]
    fn manifest_to_yaml_root_files_at_end() {
        let rules = vec![
            Rule {
                path: String::from("README.md"),
                strategy: Strategy::CreateOnly,
                source: None,
                patch: None,
            },
            Rule {
                path: String::from(".github/workflows/ci.yaml"),
                strategy: Strategy::Replace,
                source: None,
                patch: None,
            },
        ];
        let m = build_manifest("owner/repo", "main", rules);
        let yaml = manifest_to_yaml(&m).unwrap();
        let ci_pos = yaml.find(".github/workflows/ci.yaml").unwrap();
        let readme_pos = yaml.find("README.md").unwrap();
        let others_pos = yaml.find("# others").unwrap();
        assert!(
            ci_pos < others_pos,
            ".github/ group should come before # others"
        );
        assert!(
            others_pos < readme_pos,
            "# others header should appear before README.md"
        );
    }
}
