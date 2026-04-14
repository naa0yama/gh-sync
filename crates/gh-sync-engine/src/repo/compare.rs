//! Comparison logic — diff spec against live GitHub state.
// TODO: add per-item doc comments to satisfy `missing_docs` and `missing_errors_doc`
#![allow(missing_docs)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::must_use_candidate)]

use anyhow::Context as _;
use gh_sync_manifest::{
    Actions, BranchProtection, Features, Label, MergeStrategy, PullRequestRule, RefNameCondition,
    RequiredStatusChecks, Ruleset, RulesetRules, Spec,
};

use super::{
    ActionsPermissionsApi, ApiLabel, BranchProtectionApi, GhRepoClient, LiveRuleset, RepoApiData,
    SelectedActionsApi, SpecChange, WorkflowPermissionsApi,
};

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

/// Fetch live repository state via `client` and compute the diff against `spec`.
///
/// # Errors
///
/// Returns an error if any `client` fetch call fails (e.g. network error,
/// authentication failure, or the repository does not exist).
#[allow(clippy::too_many_lines)]
pub fn compare(
    spec: &Spec,
    repo: &str,
    client: &dyn GhRepoClient,
) -> anyhow::Result<Vec<SpecChange>> {
    let live = client
        .fetch_repo(repo)
        .with_context(|| format!("failed to fetch repo settings for {repo}"))?;
    let mut changes: Vec<SpecChange> = Vec::new();

    if let Some(desc) = &spec.description {
        push_str_field(
            &mut changes,
            "description",
            live.description.as_deref(),
            desc,
        );
    }
    if let Some(hp) = &spec.homepage {
        push_str_field(&mut changes, "homepage", live.homepage.as_deref(), hp);
    }
    if let Some(vis) = &spec.visibility {
        push_str_field(&mut changes, "visibility", Some(&live.visibility), vis);
    }
    if let Some(arch) = spec.archived {
        push_bool_field(&mut changes, "archived", live.archived, arch);
    }
    if let Some(v) = spec.web_commit_signoff_required {
        push_bool_field(
            &mut changes,
            "web_commit_signoff_required",
            live.web_commit_signoff_required,
            v,
        );
    }
    if let Some(ri) = spec.release_immutability {
        match client
            .fetch_release_immutability(repo)
            .with_context(|| format!("failed to fetch release immutability for {repo}"))?
        {
            Some(live_ri) => push_bool_field(&mut changes, "release_immutability", live_ri, ri),
            None => {
                changes.push(SpecChange::FieldOk {
                    field: String::from("release_immutability"),
                    value: String::from("(endpoint not available — skipped)"),
                });
            }
        }
    }

    if let Some(spec_topics) = &spec.topics {
        let live_topics = client
            .fetch_topics(repo)
            .with_context(|| format!("failed to fetch topics for {repo}"))?;
        compare_topics(&mut changes, spec_topics, &live_topics);
    }

    if let Some(feat) = &spec.features {
        compare_features(&mut changes, feat, &live);
    }

    if let Some(ms) = &spec.merge_strategy {
        compare_merge_strategy(&mut changes, ms, &live);
    }

    if spec.labels.is_some() || spec.label_sync.is_some() {
        let live_labels = client
            .fetch_labels(repo)
            .with_context(|| format!("failed to fetch labels for {repo}"))?;
        let mirror = spec.label_sync.as_deref() == Some("mirror");
        compare_labels(
            &mut changes,
            spec.labels.as_deref().unwrap_or_default(),
            &live_labels,
            mirror,
        );
    }

    if let Some(act) = &spec.actions {
        let live_perms = client
            .fetch_actions_permissions(repo)
            .with_context(|| format!("failed to fetch actions permissions for {repo}"))?;
        let live_sel = if live_perms.allowed_actions.as_deref() == Some("selected") {
            Some(
                client
                    .fetch_selected_actions(repo)
                    .with_context(|| format!("failed to fetch selected actions for {repo}"))?,
            )
        } else {
            None
        };
        let needs_wf =
            act.workflow_permissions.is_some() || act.can_approve_pull_requests.is_some();
        let live_wf = if needs_wf {
            Some(
                client
                    .fetch_workflow_permissions(repo)
                    .with_context(|| format!("failed to fetch workflow permissions for {repo}"))?,
            )
        } else {
            None
        };
        // fork_pr_approval requires its own dedicated endpoint for accurate comparison.
        let live_fork_pr = if act.fork_pr_approval.is_some() {
            client
                .fetch_fork_pr_approval(repo)
                .with_context(|| format!("failed to fetch fork PR approval for {repo}"))?
        } else {
            None
        };
        compare_actions(
            &mut changes,
            act,
            &live_perms,
            live_sel.as_ref(),
            live_wf.as_ref(),
            live_fork_pr.as_deref(),
        );
    }

    if let Some(rulesets) = &spec.rulesets {
        let live_rulesets = client
            .fetch_rulesets(repo)
            .with_context(|| format!("failed to fetch rulesets for {repo}"))?;
        compare_rulesets(&mut changes, rulesets, &live_rulesets, repo, client);
    }

    if let Some(bps) = &spec.branch_protection {
        compare_branch_protection(&mut changes, bps, repo, client)
            .with_context(|| format!("failed to compare branch protection for {repo}"))?;
    }

    Ok(changes)
}

pub(super) fn compare_topics(changes: &mut Vec<SpecChange>, spec: &[String], live: &[String]) {
    let mut spec_sorted = spec.to_vec();
    let mut live_sorted = live.to_vec();
    spec_sorted.sort();
    live_sorted.sort();
    if spec_sorted == live_sorted {
        changes.push(SpecChange::FieldOk {
            field: String::from("topics"),
            value: format!("[{}]", spec.join(", ")),
        });
    } else {
        changes.push(SpecChange::FieldChanged {
            field: String::from("topics"),
            old: format!("[{}]", live.join(", ")),
            new: format!("[{}]", spec.join(", ")),
        });
    }
}

pub(super) fn push_str_field(
    changes: &mut Vec<SpecChange>,
    field: &str,
    live: Option<&str>,
    spec: &str,
) {
    let live_val = live.unwrap_or("");
    if live_val == spec {
        changes.push(SpecChange::FieldOk {
            field: field.to_owned(),
            value: spec.to_owned(),
        });
    } else {
        changes.push(SpecChange::FieldChanged {
            field: field.to_owned(),
            old: live_val.to_owned(),
            new: spec.to_owned(),
        });
    }
}

pub(super) fn push_bool_field(changes: &mut Vec<SpecChange>, field: &str, live: bool, spec: bool) {
    if live == spec {
        changes.push(SpecChange::FieldOk {
            field: field.to_owned(),
            value: spec.to_string(),
        });
    } else {
        changes.push(SpecChange::FieldChanged {
            field: field.to_owned(),
            old: live.to_string(),
            new: spec.to_string(),
        });
    }
}

pub(super) fn compare_features(changes: &mut Vec<SpecChange>, feat: &Features, live: &RepoApiData) {
    if let Some(v) = feat.issues {
        push_bool_field(changes, "features.issues", live.has_issues, v);
    }
    if let Some(v) = feat.projects {
        push_bool_field(changes, "features.projects", live.has_projects, v);
    }
    if let Some(v) = feat.wiki {
        push_bool_field(changes, "features.wiki", live.has_wiki, v);
    }
    if let Some(v) = feat.discussions {
        push_bool_field(changes, "features.discussions", live.has_discussions, v);
    }
}

pub(super) fn compare_merge_strategy(
    changes: &mut Vec<SpecChange>,
    ms: &MergeStrategy,
    live: &RepoApiData,
) {
    if let Some(v) = ms.allow_merge_commit {
        push_bool_field(
            changes,
            "merge_strategy.allow_merge_commit",
            live.allow_merge_commit,
            v,
        );
    }
    if let Some(v) = ms.allow_squash_merge {
        push_bool_field(
            changes,
            "merge_strategy.allow_squash_merge",
            live.allow_squash_merge,
            v,
        );
    }
    if let Some(v) = ms.allow_rebase_merge {
        push_bool_field(
            changes,
            "merge_strategy.allow_rebase_merge",
            live.allow_rebase_merge,
            v,
        );
    }
    if let Some(v) = ms.allow_auto_merge {
        push_bool_field(
            changes,
            "merge_strategy.allow_auto_merge",
            live.allow_auto_merge,
            v,
        );
    }
    if let Some(v) = ms.allow_update_branch {
        push_bool_field(
            changes,
            "merge_strategy.allow_update_branch",
            live.allow_update_branch,
            v,
        );
    }
    if let Some(v) = ms.auto_delete_head_branches {
        push_bool_field(
            changes,
            "merge_strategy.auto_delete_head_branches",
            live.delete_branch_on_merge,
            v,
        );
    }
    if let Some(v) = &ms.squash_merge_commit_title {
        push_str_field(
            changes,
            "merge_strategy.squash_merge_commit_title",
            live.squash_merge_commit_title.as_deref(),
            v,
        );
    }
    if let Some(v) = &ms.squash_merge_commit_message {
        push_str_field(
            changes,
            "merge_strategy.squash_merge_commit_message",
            live.squash_merge_commit_message.as_deref(),
            v,
        );
    }
    if let Some(v) = &ms.merge_commit_title {
        push_str_field(
            changes,
            "merge_strategy.merge_commit_title",
            live.merge_commit_title.as_deref(),
            v,
        );
    }
    if let Some(v) = &ms.merge_commit_message {
        push_str_field(
            changes,
            "merge_strategy.merge_commit_message",
            live.merge_commit_message.as_deref(),
            v,
        );
    }
}

/// Normalise a label colour to lowercase hex without the leading `#`.
pub(super) fn normalize_color(color: &str) -> String {
    color.trim_start_matches('#').to_lowercase()
}

fn compare_labels(
    changes: &mut Vec<SpecChange>,
    spec_labels: &[Label],
    live_labels: &[ApiLabel],
    mirror: bool,
) {
    for sl in spec_labels {
        let color = normalize_color(&sl.color);
        match live_labels.iter().find(|l| l.name == sl.name) {
            None => {
                changes.push(SpecChange::LabelAdd {
                    name: sl.name.clone(),
                    color,
                    description: sl.description.clone(),
                });
            }
            Some(ll) => {
                let live_color = normalize_color(&ll.color);
                if live_color == color && ll.description == sl.description {
                    changes.push(SpecChange::LabelOk {
                        name: sl.name.clone(),
                    });
                } else {
                    changes.push(SpecChange::LabelUpdate {
                        name: sl.name.clone(),
                        old_color: live_color,
                        old_description: ll.description.clone(),
                        new_color: color,
                        new_description: sl.description.clone(),
                    });
                }
            }
        }
    }
    if mirror {
        for ll in live_labels {
            if !spec_labels.iter().any(|sl| sl.name == ll.name) {
                changes.push(SpecChange::LabelDelete {
                    name: ll.name.clone(),
                });
            }
        }
    }
}

pub(super) fn compare_actions(
    changes: &mut Vec<SpecChange>,
    act: &Actions,
    live: &ActionsPermissionsApi,
    live_sel: Option<&SelectedActionsApi>,
    live_wf: Option<&WorkflowPermissionsApi>,
    live_fork_pr: Option<&str>,
) {
    if let Some(v) = act.enabled {
        push_bool_field(changes, "actions.enabled", live.enabled, v);
    }
    if let Some(v) = &act.allowed_actions {
        push_str_field(
            changes,
            "actions.allowed_actions",
            live.allowed_actions.as_deref(),
            v,
        );
    }
    if let Some(sel) = &act.selected_actions
        && let Some(live_sel) = live_sel
    {
        if let Some(v) = sel.github_owned_allowed {
            push_bool_field(
                changes,
                "actions.selected_actions.github_owned_allowed",
                live_sel.github_owned_allowed.unwrap_or(false),
                v,
            );
        }
        if let Some(patterns) = &sel.patterns_allowed {
            let mut spec_p = patterns.clone();
            let mut live_p = live_sel.patterns_allowed.clone().unwrap_or_default();
            spec_p.sort();
            live_p.sort();
            if spec_p == live_p {
                changes.push(SpecChange::FieldOk {
                    field: String::from("actions.selected_actions.patterns_allowed"),
                    value: format!("[{}]", patterns.join(", ")),
                });
            } else {
                changes.push(SpecChange::FieldChanged {
                    field: String::from("actions.selected_actions.patterns_allowed"),
                    old: format!("[{}]", live_p.join(", ")),
                    new: format!("[{}]", patterns.join(", ")),
                });
            }
        }
    }
    if let Some(v) = &act.workflow_permissions
        && let Some(wf) = live_wf
    {
        push_str_field(
            changes,
            "actions.workflow_permissions",
            wf.default_workflow_permissions.as_deref(),
            v,
        );
    }
    if let Some(v) = act.can_approve_pull_requests
        && let Some(wf) = live_wf
    {
        push_bool_field(
            changes,
            "actions.can_approve_pull_requests",
            wf.can_approve_pull_request_reviews,
            v,
        );
    }
    if let Some(ref v) = act.fork_pr_approval {
        // live_fork_pr is fetched from the dedicated endpoint; None means the
        // endpoint is unavailable → skip comparison.
        if let Some(live_v) = live_fork_pr {
            push_str_field(changes, "actions.fork_pr_approval", Some(live_v), v);
        } else {
            changes.push(SpecChange::FieldOk {
                field: String::from("actions.fork_pr_approval"),
                value: String::from("(endpoint not available — skipped)"),
            });
        }
    }
    if let Some(v) = act.sha_pinning_required {
        push_bool_field(
            changes,
            "actions.sha_pinning_required",
            live.sha_pinning_required.unwrap_or(false),
            v,
        );
    }
}

/// Compare rulesets by name: ADD for new, OK for unchanged, UPDATE for changed, DELETE for removed.
///
/// Fetches live ruleset details to compare content field-by-field, so applying
/// the same spec twice produces `RulesetOk` on the second run.  If fetching
/// details fails (network error, permission) the ruleset is conservatively
/// treated as changed.
fn compare_rulesets(
    changes: &mut Vec<SpecChange>,
    spec_rulesets: &[Ruleset],
    live_rulesets: &[LiveRuleset],
    repo: &str,
    client: &dyn GhRepoClient,
) {
    for sr in spec_rulesets {
        match live_rulesets.iter().find(|lr| lr.name == sr.name) {
            None => {
                changes.push(SpecChange::RulesetAdd {
                    name: sr.name.clone(),
                    spec: Box::new(sr.clone()),
                });
            }
            Some(lr) => {
                let (needs_update, live_json_opt) = match client.fetch_ruleset_details(repo, lr.id)
                {
                    Ok(live_json) => {
                        let is_changed = !ruleset_matches(sr, &live_json);
                        (is_changed, Some(live_json))
                    }
                    Err(e) => {
                        tracing::debug!(
                            ruleset = %sr.name,
                            id = lr.id,
                            "failed to fetch ruleset details: {e} — treating as update"
                        );
                        (true, None)
                    }
                };
                if needs_update {
                    changes.push(SpecChange::RulesetUpdate {
                        id: lr.id,
                        name: sr.name.clone(),
                        spec: Box::new(sr.clone()),
                        live: live_json_opt.map(Box::new),
                    });
                } else {
                    changes.push(SpecChange::RulesetOk {
                        id: lr.id,
                        name: sr.name.clone(),
                    });
                }
            }
        }
    }
    for lr in live_rulesets {
        if !spec_rulesets.iter().any(|sr| sr.name == lr.name) {
            changes.push(SpecChange::RulesetDelete {
                id: lr.id,
                name: lr.name.clone(),
            });
        }
    }
}

/// Return `true` when the spec ruleset matches the live GitHub API JSON.
///
/// Compares target, enforcement, conditions, and rules field-by-field.
/// `bypass_actors` are skipped (ID resolution would require extra API calls).
/// For `required_status_checks`, only the `context` string is compared —
/// `integration_id` is resolved at apply-time and not available here.
pub(super) fn ruleset_matches(spec: &Ruleset, live: &serde_json::Value) -> bool {
    // target / enforcement
    let spec_target = spec.target.as_deref().unwrap_or("branch");
    if live.get("target").and_then(serde_json::Value::as_str) != Some(spec_target) {
        return false;
    }
    let spec_enforcement = spec.enforcement.as_deref().unwrap_or("active");
    if live.get("enforcement").and_then(serde_json::Value::as_str) != Some(spec_enforcement) {
        return false;
    }

    // conditions
    if let Some(cond) = &spec.conditions
        && let Some(ref_name) = &cond.ref_name
        && !ruleset_conditions_match(ref_name, live)
    {
        return false;
    }

    // rules
    if let Some(rules) = &spec.rules {
        let live_rules: &[serde_json::Value] = live
            .get("rules")
            .and_then(serde_json::Value::as_array)
            .map_or(&[], Vec::as_slice);
        if !ruleset_rules_match(rules, live_rules) {
            return false;
        }
    }

    true
}

pub(super) fn ruleset_conditions_match(
    ref_name: &RefNameCondition,
    live: &serde_json::Value,
) -> bool {
    let live_rn = live.get("conditions").and_then(|c| c.get("ref_name"));
    let live_include: Vec<&str> = live_rn
        .and_then(|rn| rn.get("include"))
        .and_then(serde_json::Value::as_array)
        .map(|a| a.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    let live_exclude: Vec<&str> = live_rn
        .and_then(|rn| rn.get("exclude"))
        .and_then(serde_json::Value::as_array)
        .map(|a| a.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    let spec_include: Vec<&str> = ref_name
        .include
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(String::as_str)
        .collect();
    let spec_exclude: Vec<&str> = ref_name
        .exclude
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(String::as_str)
        .collect();

    let mut si = spec_include;
    let mut li = live_include;
    let mut se = spec_exclude;
    let mut le = live_exclude;
    si.sort_unstable();
    li.sort_unstable();
    se.sort_unstable();
    le.sort_unstable();
    si == li && se == le
}

pub(super) fn ruleset_rules_match(rules: &RulesetRules, live_rules: &[serde_json::Value]) -> bool {
    let live_types: std::collections::HashSet<&str> = live_rules
        .iter()
        .filter_map(|r| r.get("type").and_then(serde_json::Value::as_str))
        .collect();

    // Simple boolean rules: present in live array ↔ enabled in spec.
    for (name, spec_val) in [
        ("non_fast_forward", rules.non_fast_forward),
        ("deletion", rules.deletion),
        ("creation", rules.creation),
        ("required_linear_history", rules.required_linear_history),
        ("required_signatures", rules.required_signatures),
    ] {
        if let Some(v) = spec_val
            && live_types.contains(name) != v
        {
            return false;
        }
    }

    if let Some(spec_pr) = &rules.pull_request {
        let live_pr = live_rules
            .iter()
            .find(|r| r.get("type").and_then(serde_json::Value::as_str) == Some("pull_request"))
            .and_then(|r| r.get("parameters"));
        if !ruleset_pr_matches(spec_pr, live_pr) {
            return false;
        }
    }

    if let Some(spec_sc) = &rules.required_status_checks {
        let live_sc = live_rules
            .iter()
            .find(|r| {
                r.get("type").and_then(serde_json::Value::as_str) == Some("required_status_checks")
            })
            .and_then(|r| r.get("parameters"));
        if !ruleset_status_checks_match(spec_sc, live_sc) {
            return false;
        }
    }

    true
}

pub(super) fn ruleset_pr_matches(
    spec_pr: &PullRequestRule,
    live_params: Option<&serde_json::Value>,
) -> bool {
    let Some(lp) = live_params else {
        return false;
    };
    let check_u64 = |key: &str, spec_opt: Option<u64>| -> bool {
        spec_opt.is_none_or(|v| lp.get(key).and_then(serde_json::Value::as_u64) == Some(v))
    };
    let check_bool = |key: &str, spec_opt: Option<bool>| -> bool {
        spec_opt.is_none_or(|v| lp.get(key).and_then(serde_json::Value::as_bool) == Some(v))
    };
    let methods_match = spec_pr
        .allowed_merge_methods
        .as_ref()
        .is_none_or(|spec_methods| {
            let live_methods: Vec<&str> = lp
                .get("allowed_merge_methods")
                .and_then(serde_json::Value::as_array)
                .map(|a| a.iter().filter_map(serde_json::Value::as_str).collect())
                .unwrap_or_default();
            let mut s: Vec<&str> = spec_methods.iter().map(String::as_str).collect();
            let mut l: Vec<&str> = live_methods;
            s.sort_unstable();
            l.sort_unstable();
            s == l
        });
    check_u64(
        "required_approving_review_count",
        spec_pr.required_approving_review_count.map(u64::from),
    ) && check_bool(
        "dismiss_stale_reviews_on_push",
        spec_pr.dismiss_stale_reviews_on_push,
    ) && check_bool(
        "require_code_owner_review",
        spec_pr.require_code_owner_review,
    ) && check_bool(
        "require_last_push_approval",
        spec_pr.require_last_push_approval,
    ) && check_bool(
        "required_review_thread_resolution",
        spec_pr.required_review_thread_resolution,
    ) && methods_match
}

pub(super) fn ruleset_status_checks_match(
    spec_sc: &RequiredStatusChecks,
    live_params: Option<&serde_json::Value>,
) -> bool {
    let Some(lsc) = live_params else {
        return false;
    };
    if let Some(v) = spec_sc.strict_required_status_checks_policy
        && lsc
            .get("strict_required_status_checks_policy")
            .and_then(serde_json::Value::as_bool)
            != Some(v)
    {
        return false;
    }
    if let Some(spec_ctxs) = &spec_sc.contexts {
        let live_ctx_names: Vec<&str> = lsc
            .get("required_status_checks")
            .and_then(serde_json::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|c| c.get("context").and_then(serde_json::Value::as_str))
                    .collect()
            })
            .unwrap_or_default();
        let mut spec_names: Vec<&str> = spec_ctxs.iter().map(|c| c.context.as_str()).collect();
        let mut live_names = live_ctx_names;
        spec_names.sort_unstable();
        live_names.sort_unstable();
        if spec_names != live_names {
            return false;
        }
    }
    true
}

fn compare_branch_protection(
    changes: &mut Vec<SpecChange>,
    spec_bps: &[BranchProtection],
    repo: &str,
    client: &dyn GhRepoClient,
) -> anyhow::Result<()> {
    let spec_patterns: std::collections::HashSet<&str> =
        spec_bps.iter().map(|bp| bp.pattern.as_str()).collect();

    for sbp in spec_bps {
        let live = client
            .fetch_branch_protection(repo, &sbp.pattern)
            .with_context(|| {
                format!(
                    "failed to fetch branch protection for {repo}/{}",
                    sbp.pattern
                )
            })?;
        match live {
            None => {
                changes.push(SpecChange::BranchProtectionAdd {
                    spec: Box::new(sbp.clone()),
                });
            }
            Some(ref l) => {
                if protection_matches(sbp, l) {
                    changes.push(SpecChange::BranchProtectionOk {
                        pattern: sbp.pattern.clone(),
                    });
                } else {
                    changes.push(SpecChange::BranchProtectionUpdate {
                        spec: Box::new(sbp.clone()),
                    });
                }
            }
        }
    }

    // Find protected branches not in spec → schedule for removal.
    // Note: list_protected_branches returns exact branch names only;
    // wildcard-pattern rules cannot be discovered via the REST API.
    let live_branches = client
        .list_protected_branches(repo)
        .with_context(|| format!("failed to list protected branches for {repo}"))?;
    for pattern in live_branches {
        if !spec_patterns.contains(pattern.as_str()) {
            changes.push(SpecChange::BranchProtectionRemove { pattern });
        }
    }

    Ok(())
}

pub(super) fn protection_matches(spec: &BranchProtection, live: &BranchProtectionApi) -> bool {
    if let Some(v) = spec.required_reviews
        && live.required_reviews != Some(v)
    {
        return false;
    }
    if let Some(v) = spec.dismiss_stale_reviews
        && live.dismiss_stale_reviews != v
    {
        return false;
    }
    if let Some(v) = spec.require_code_owner_reviews
        && live.require_code_owner_reviews != v
    {
        return false;
    }
    if let Some(sc) = &spec.require_status_checks {
        if let Some(strict) = sc.strict
            && live.strict_status_checks != strict
        {
            return false;
        }
        if let Some(ctxs) = &sc.contexts {
            let mut spec_ctxs: Vec<&str> = ctxs.iter().map(String::as_str).collect();
            let mut live_ctxs: Vec<&str> = live
                .status_check_contexts
                .iter()
                .map(String::as_str)
                .collect();
            spec_ctxs.sort_unstable();
            live_ctxs.sort_unstable();
            if spec_ctxs != live_ctxs {
                return false;
            }
        }
    }
    if let Some(v) = spec.enforce_admins
        && live.enforce_admins != v
    {
        return false;
    }
    if let Some(v) = spec.allow_force_pushes
        && live.allow_force_pushes != v
    {
        return false;
    }
    if let Some(v) = spec.allow_deletions
        && live.allow_deletions != v
    {
        return false;
    }
    true
}
