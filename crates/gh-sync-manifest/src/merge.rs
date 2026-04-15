//! Merging an upstream manifest with a local overlay.
//!
//! The local overlay wins on a per-field / per-path basis; entries the local
//! manifest does not mention are inherited verbatim from upstream.

use crate::manifest::{Manifest, Rule, Spec, Strategy};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Merge a local overlay manifest on top of an upstream manifest.
///
/// Merge rules:
///
/// - `upstream:` node — local replaces upstream entirely when present.
/// - `spec:` node — field-by-field merge; local `Some` fields win.
///   Nested option fields (`features`, `merge_strategy`, `actions`, etc.)
///   are replaced at the node level if local defines them.
/// - `files:` — merged by `path` key:
///   - upstream-only entries are kept as-is.
///   - local-only entries are appended.
///   - entries present in both are replaced by the local rule.
///   - a local entry with `strategy: ignore` **removes** the upstream entry
///     (the path is dropped from the result entirely).
///
/// # Returns
///
/// Returns `upstream` unchanged when `local` is `None`.
#[must_use]
#[allow(clippy::module_name_repetitions)] // "merge_overlay" in module "merge" is intentional
pub fn merge_overlay(upstream: Manifest, local: Option<Manifest>) -> Manifest {
    let Some(local) = local else {
        return upstream;
    };

    let merged_upstream = local.upstream;
    let merged_spec = merge_spec(upstream.spec, local.spec);
    let merged_files = merge_files(upstream.files, local.files);

    Manifest {
        upstream: merged_upstream,
        spec: merged_spec,
        files: merged_files,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn merge_spec(upstream: Option<Spec>, local: Option<Spec>) -> Option<Spec> {
    match (upstream, local) {
        (None, None) => None,
        (Some(u), None) => Some(u),
        (None, Some(l)) => Some(l),
        (Some(u), Some(l)) => Some(Spec {
            description: l.description.or(u.description),
            homepage: l.homepage.or(u.homepage),
            visibility: l.visibility.or(u.visibility),
            archived: l.archived.or(u.archived),
            topics: l.topics.or(u.topics),
            features: l.features.or(u.features),
            web_commit_signoff_required: l
                .web_commit_signoff_required
                .or(u.web_commit_signoff_required),
            merge_strategy: l.merge_strategy.or(u.merge_strategy),
            release_immutability: l.release_immutability.or(u.release_immutability),
            label_sync: l.label_sync.or(u.label_sync),
            labels: l.labels.or(u.labels),
            actions: l.actions.or(u.actions),
            rulesets: l.rulesets.or(u.rulesets),
            branch_protection: l.branch_protection.or(u.branch_protection),
        }),
    }
}

/// Merge two `files:` lists by `path` key.
///
/// The order is: upstream rules (overridden or dropped by local), then
/// local-only rules (not present in upstream).
fn merge_files(upstream: Vec<Rule>, local: Vec<Rule>) -> Vec<Rule> {
    let capacity = upstream.len().saturating_add(local.len());
    let mut result: Vec<Rule> = Vec::with_capacity(capacity);

    for up_rule in upstream {
        // Look for a matching local rule by path.
        if let Some(local_rule) = local.iter().find(|r| r.path == up_rule.path) {
            // `ignore` drops the upstream entry entirely.
            if local_rule.strategy != Strategy::Ignore {
                result.push(Rule {
                    path: local_rule.path.clone(),
                    strategy: local_rule.strategy,
                    source: local_rule.source.clone(),
                    patch: local_rule.patch.clone(),
                });
            }
        } else {
            result.push(up_rule);
        }
    }

    // Append local-only rules (paths not present in upstream).
    for local_rule in local {
        let already_seen = result.iter().any(|r| r.path == local_rule.path);
        if !already_seen && local_rule.strategy != Strategy::Ignore {
            result.push(local_rule);
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::indexing_slicing)] // test assertions on known-size vecs

    use super::*;
    use crate::manifest::{Spec, Strategy, Upstream};

    fn upstream_rule(path: &str, strategy: Strategy) -> Rule {
        Rule {
            path: path.to_owned(),
            strategy,
            source: None,
            patch: None,
        }
    }

    fn local_rule(path: &str, strategy: Strategy) -> Rule {
        Rule {
            path: path.to_owned(),
            strategy,
            source: None,
            patch: None,
        }
    }

    fn make_manifest(files: Vec<Rule>) -> Manifest {
        Manifest {
            upstream: Upstream {
                repo: String::from("owner/repo"),
                ref_: String::from("main"),
            },
            spec: None,
            files,
        }
    }

    fn make_manifest_with_upstream(upstream_repo: &str, files: Vec<Rule>) -> Manifest {
        Manifest {
            upstream: Upstream {
                repo: upstream_repo.to_owned(),
                ref_: String::from("main"),
            },
            spec: None,
            files,
        }
    }

    // --- No local overlay -------------------------------------------------------

    #[test]
    fn no_local_returns_upstream_unchanged() {
        let up = make_manifest(vec![upstream_rule("foo.txt", Strategy::Replace)]);
        let merged = merge_overlay(up, None);
        assert_eq!(merged.files.len(), 1);
        assert_eq!(merged.files[0].path, "foo.txt");
        assert_eq!(merged.upstream.repo, "owner/repo");
    }

    // --- files: merge -----------------------------------------------------------

    #[test]
    fn upstream_only_files_are_kept() {
        let up = make_manifest(vec![upstream_rule("a.txt", Strategy::Replace)]);
        let local = make_manifest(vec![]);
        let merged = merge_overlay(up, Some(local));
        assert_eq!(merged.files.len(), 1);
        assert_eq!(merged.files[0].path, "a.txt");
    }

    #[test]
    fn local_only_files_are_appended() {
        let up = make_manifest(vec![]);
        let local = make_manifest(vec![local_rule("b.txt", Strategy::CreateOnly)]);
        let merged = merge_overlay(up, Some(local));
        assert_eq!(merged.files.len(), 1);
        assert_eq!(merged.files[0].path, "b.txt");
        assert_eq!(merged.files[0].strategy, Strategy::CreateOnly);
    }

    #[test]
    fn local_wins_on_same_path() {
        let up = make_manifest(vec![upstream_rule("Cargo.toml", Strategy::Replace)]);
        let local = make_manifest(vec![local_rule("Cargo.toml", Strategy::Patch)]);
        let merged = merge_overlay(up, Some(local));
        assert_eq!(merged.files.len(), 1);
        assert_eq!(merged.files[0].strategy, Strategy::Patch);
    }

    #[test]
    fn ignore_drops_upstream_entry() {
        let up = make_manifest(vec![
            upstream_rule("keep.txt", Strategy::Replace),
            upstream_rule("drop.txt", Strategy::Replace),
        ]);
        let local = make_manifest(vec![local_rule("drop.txt", Strategy::Ignore)]);
        let merged = merge_overlay(up, Some(local));
        assert_eq!(merged.files.len(), 1);
        assert_eq!(merged.files[0].path, "keep.txt");
    }

    #[test]
    fn ignore_local_only_is_not_appended() {
        // A local `ignore` for a path not in upstream should not appear in result.
        let up = make_manifest(vec![upstream_rule("a.txt", Strategy::Replace)]);
        let local = make_manifest(vec![local_rule("nonexistent.txt", Strategy::Ignore)]);
        let merged = merge_overlay(up, Some(local));
        assert_eq!(merged.files.len(), 1);
        assert_eq!(merged.files[0].path, "a.txt");
    }

    #[test]
    fn mixed_files_merge_correctly() {
        let up = make_manifest(vec![
            upstream_rule("a.txt", Strategy::Replace),
            upstream_rule("b.txt", Strategy::Replace),
            upstream_rule("c.txt", Strategy::Replace),
        ]);
        let local = make_manifest(vec![
            local_rule("b.txt", Strategy::Patch),      // override
            local_rule("c.txt", Strategy::Ignore),     // drop
            local_rule("d.txt", Strategy::CreateOnly), // local-only
        ]);
        let merged = merge_overlay(up, Some(local));
        // Result: a.txt (upstream), b.txt (local patch), d.txt (local-only)
        assert_eq!(merged.files.len(), 3);
        assert_eq!(merged.files[0].path, "a.txt");
        assert_eq!(merged.files[0].strategy, Strategy::Replace);
        assert_eq!(merged.files[1].path, "b.txt");
        assert_eq!(merged.files[1].strategy, Strategy::Patch);
        assert_eq!(merged.files[2].path, "d.txt");
        assert_eq!(merged.files[2].strategy, Strategy::CreateOnly);
    }

    // --- upstream: node ---------------------------------------------------------

    #[test]
    fn local_upstream_node_replaces_upstream() {
        let up = make_manifest_with_upstream("upstream/template", vec![]);
        let local = make_manifest_with_upstream("local/override", vec![]);
        let merged = merge_overlay(up, Some(local));
        assert_eq!(merged.upstream.repo, "local/override");
    }

    // --- spec: field-wise merge -------------------------------------------------

    #[test]
    fn spec_local_wins_per_field() {
        let mut up = make_manifest(vec![upstream_rule("f.txt", Strategy::Replace)]);
        up.spec = Some(Spec {
            description: Some(String::from("upstream desc")),
            homepage: Some(String::from("https://upstream.example")),
            visibility: Some(String::from("private")),
            archived: None,
            topics: None,
            features: None,
            web_commit_signoff_required: None,
            merge_strategy: None,
            release_immutability: None,
            label_sync: None,
            labels: None,
            actions: None,
            rulesets: None,
            branch_protection: None,
        });

        let mut local = make_manifest(vec![]);
        local.spec = Some(Spec {
            description: Some(String::from("local desc")),
            homepage: None,
            visibility: None,
            archived: Some(false),
            topics: None,
            features: None,
            web_commit_signoff_required: None,
            merge_strategy: None,
            release_immutability: None,
            label_sync: None,
            labels: None,
            actions: None,
            rulesets: None,
            branch_protection: None,
        });

        let merged = merge_overlay(up, Some(local));
        let spec = merged.spec.unwrap();
        // local description wins
        assert_eq!(spec.description.as_deref(), Some("local desc"));
        // upstream homepage is preserved (local had None)
        assert_eq!(spec.homepage.as_deref(), Some("https://upstream.example"));
        // upstream visibility preserved
        assert_eq!(spec.visibility.as_deref(), Some("private"));
        // local archived wins
        assert_eq!(spec.archived, Some(false));
    }

    #[test]
    fn spec_upstream_only_preserved() {
        let mut up = make_manifest(vec![upstream_rule("f.txt", Strategy::Replace)]);
        up.spec = Some(Spec {
            description: Some(String::from("desc")),
            homepage: None,
            visibility: None,
            archived: None,
            topics: None,
            features: None,
            web_commit_signoff_required: None,
            merge_strategy: None,
            release_immutability: None,
            label_sync: None,
            labels: None,
            actions: None,
            rulesets: None,
            branch_protection: None,
        });

        let local = make_manifest(vec![]);
        let merged = merge_overlay(up, Some(local));
        let spec = merged.spec.unwrap();
        assert_eq!(spec.description.as_deref(), Some("desc"));
    }

    #[test]
    fn spec_both_none_stays_none() {
        let up = make_manifest(vec![upstream_rule("f.txt", Strategy::Replace)]);
        let local = make_manifest(vec![]);
        let merged = merge_overlay(up, Some(local));
        assert!(merged.spec.is_none());
    }
}
