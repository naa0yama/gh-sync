//! Apply pending spec changes to the live repository.
// TODO: add per-item doc comments to satisfy `missing_docs` and `missing_errors_doc`
#![allow(missing_docs)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::must_use_candidate)]

use gh_sync_manifest::{
    BranchProtection, BypassActor, Ruleset, RulesetRules, SelectedActions, Spec,
};

use super::{GhRepoClient, SpecChange};

// ---------------------------------------------------------------------------
// Apply
// ---------------------------------------------------------------------------

/// Apply all pending `changes` to the live repository via `client`.
///
/// # Errors
///
/// Returns an error if any API write call fails (e.g. insufficient permissions,
/// network error, or invalid field values).
pub fn apply_changes(
    changes: &[SpecChange],
    spec: &Spec,
    repo: &str,
    client: &dyn GhRepoClient,
) -> anyhow::Result<()> {
    apply_core_fields(changes, spec, repo, client)?;
    apply_label_changes(changes, repo, client)?;
    apply_actions_changes(changes, spec, repo, client)?;
    apply_ruleset_changes(changes, repo, client)?;
    apply_branch_protection_changes(changes, repo, client)?;
    Ok(())
}

/// Map a `SpecChange::FieldChanged` field name to a GitHub API PATCH key and value.
///
/// Returns `None` for `topics` (handled separately) and unknown fields.
pub(super) fn core_field_to_patch(
    field: &str,
    new: &str,
) -> Option<(&'static str, serde_json::Value)> {
    let b = new == "true";
    match field {
        "description" => Some(("description", serde_json::json!(new))),
        "homepage" => Some(("homepage", serde_json::json!(new))),
        "visibility" => Some(("visibility", serde_json::json!(new))),
        "archived" => Some(("archived", serde_json::json!(b))),
        "features.issues" => Some(("has_issues", serde_json::json!(b))),
        "features.projects" => Some(("has_projects", serde_json::json!(b))),
        "features.wiki" => Some(("has_wiki", serde_json::json!(b))),
        "features.discussions" => Some(("has_discussions", serde_json::json!(b))),
        "web_commit_signoff_required" => {
            Some(("web_commit_signoff_required", serde_json::json!(b)))
        }
        "merge_strategy.allow_auto_merge" => Some(("allow_auto_merge", serde_json::json!(b))),
        "merge_strategy.allow_update_branch" => Some(("allow_update_branch", serde_json::json!(b))),
        "merge_strategy.allow_merge_commit" => Some(("allow_merge_commit", serde_json::json!(b))),
        "merge_strategy.allow_squash_merge" => Some(("allow_squash_merge", serde_json::json!(b))),
        "merge_strategy.allow_rebase_merge" => Some(("allow_rebase_merge", serde_json::json!(b))),
        "merge_strategy.auto_delete_head_branches" => {
            Some(("delete_branch_on_merge", serde_json::json!(b)))
        }
        "merge_strategy.squash_merge_commit_title" => {
            Some(("squash_merge_commit_title", serde_json::json!(new)))
        }
        "merge_strategy.squash_merge_commit_message" => {
            Some(("squash_merge_commit_message", serde_json::json!(new)))
        }
        "merge_strategy.merge_commit_title" => Some(("merge_commit_title", serde_json::json!(new))),
        "merge_strategy.merge_commit_message" => {
            Some(("merge_commit_message", serde_json::json!(new)))
        }
        _ => None,
    }
}

fn apply_core_fields(
    changes: &[SpecChange],
    spec: &Spec,
    repo: &str,
    client: &dyn GhRepoClient,
) -> anyhow::Result<()> {
    let mut patch: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    let mut topics_changed = false;

    for change in changes {
        if let SpecChange::FieldChanged { field, new, .. } = change {
            if field == "topics" {
                topics_changed = true;
            } else if let Some((key, val)) = core_field_to_patch(field, new) {
                patch.insert(key.to_owned(), val);
            }
        }
    }

    // GitHub API rejects merge-method title/message fields when the method is
    // disabled. Strip them when the same PATCH disables the method.
    if patch.get("allow_merge_commit") == Some(&serde_json::json!(false)) {
        patch.remove("merge_commit_title");
        patch.remove("merge_commit_message");
    }
    if patch.get("allow_squash_merge") == Some(&serde_json::json!(false)) {
        patch.remove("squash_merge_commit_title");
        patch.remove("squash_merge_commit_message");
    }

    if !patch.is_empty() {
        tracing::info!("applying repository settings");
        client.patch_repo(repo, &serde_json::Value::Object(patch))?;
    }

    if topics_changed && let Some(topics) = &spec.topics {
        tracing::info!("applying topics");
        client.put_topics(repo, topics)?;
    }

    // release_immutability uses a separate endpoint (PUT/DELETE).
    apply_release_immutability_change(changes, repo, client)?;

    Ok(())
}

fn apply_release_immutability_change(
    changes: &[SpecChange],
    repo: &str,
    client: &dyn GhRepoClient,
) -> anyhow::Result<()> {
    for change in changes {
        if let SpecChange::FieldChanged { field, new, .. } = change
            && field == "release_immutability"
        {
            let enabled = new == "true";
            tracing::info!("applying release_immutability: {enabled}");
            client.put_release_immutability(repo, enabled)?;
        }
    }
    Ok(())
}

fn apply_label_changes(
    changes: &[SpecChange],
    repo: &str,
    client: &dyn GhRepoClient,
) -> anyhow::Result<()> {
    for change in changes {
        match change {
            SpecChange::LabelAdd {
                name,
                color,
                description,
            } => {
                tracing::info!("creating label: {name}");
                client.create_label(repo, name, color, description.as_deref())?;
            }
            SpecChange::LabelUpdate {
                name,
                new_color,
                new_description,
                ..
            } => {
                tracing::info!("updating label: {name}");
                client.update_label(repo, name, new_color, new_description.as_deref())?;
            }
            SpecChange::LabelDelete { name } => {
                tracing::info!("deleting label: {name}");
                client.delete_label(repo, name)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn apply_selected_actions_body(
    sel: &SelectedActions,
    repo: &str,
    client: &dyn GhRepoClient,
) -> anyhow::Result<()> {
    let mut sel_body = serde_json::Map::new();
    if let Some(v) = sel.github_owned_allowed {
        sel_body.insert(String::from("github_owned_allowed"), serde_json::json!(v));
    }
    if let Some(patterns) = &sel.patterns_allowed {
        sel_body.insert(
            String::from("patterns_allowed"),
            serde_json::json!(patterns),
        );
    }
    if !sel_body.is_empty() {
        client.put_selected_actions(repo, &serde_json::Value::Object(sel_body))?;
    }
    Ok(())
}

fn apply_actions_changes(
    changes: &[SpecChange],
    spec: &Spec,
    repo: &str,
    client: &dyn GhRepoClient,
) -> anyhow::Result<()> {
    // Actions permissions (enabled, allowed_actions, sha_pinning_required)
    let actions_perms_changed = changes.iter().any(|c| {
        matches!(c, SpecChange::FieldChanged { field, .. }
            if matches!(field.as_str(),
                "actions.enabled"
                | "actions.allowed_actions"
                | "actions.sha_pinning_required"))
    });
    if actions_perms_changed && let Some(act) = &spec.actions {
        tracing::info!("applying actions permissions");
        let mut body = serde_json::Map::new();
        if let Some(enabled) = act.enabled {
            body.insert(String::from("enabled"), serde_json::json!(enabled));
        }
        if let Some(aa) = &act.allowed_actions {
            body.insert(String::from("allowed_actions"), serde_json::json!(aa));
        }
        if let Some(v) = act.sha_pinning_required {
            body.insert(String::from("sha_pinning_required"), serde_json::json!(v));
        }
        client.put_actions_permissions(repo, &serde_json::Value::Object(body))?;

        if let Some(sel) = &act.selected_actions {
            apply_selected_actions_body(sel, repo, client)?;
        }
    }

    // fork_pr_approval requires its own endpoint separate from the main permissions API
    let fork_pr_approval_changed = changes.iter().any(|c| {
        matches!(c, SpecChange::FieldChanged { field, .. } if field == "actions.fork_pr_approval")
    });
    if fork_pr_approval_changed
        && let Some(act) = &spec.actions
        && let Some(ref policy) = act.fork_pr_approval
    {
        tracing::info!("applying fork_pr_approval via dedicated endpoint");
        client.put_fork_pr_approval(repo, policy)?;
    }

    // Apply selected-actions separately when only the allowlist changed (not covered by actions_perms)
    let selected_actions_changed = changes.iter().any(|c| {
        matches!(c, SpecChange::FieldChanged { field, .. }
            if field.starts_with("actions.selected_actions"))
    });
    if selected_actions_changed
        && !actions_perms_changed
        && let Some(act) = &spec.actions
        && let Some(sel) = &act.selected_actions
    {
        tracing::info!("applying selected actions");
        apply_selected_actions_body(sel, repo, client)?;
    }

    // Workflow permissions
    let wf_changed = changes.iter().any(|c| {
        matches!(c, SpecChange::FieldChanged { field, .. }
            if field == "actions.workflow_permissions"
            || field == "actions.can_approve_pull_requests")
    });
    if wf_changed && let Some(act) = &spec.actions {
        let mut wf_body = serde_json::Map::new();
        if let Some(v) = &act.workflow_permissions {
            wf_body.insert(
                String::from("default_workflow_permissions"),
                serde_json::json!(v),
            );
        }
        if let Some(v) = act.can_approve_pull_requests {
            wf_body.insert(
                String::from("can_approve_pull_request_reviews"),
                serde_json::json!(v),
            );
        }
        if !wf_body.is_empty() {
            tracing::info!("applying workflow permissions");
            client.put_workflow_permissions(repo, &serde_json::Value::Object(wf_body))?;
        }
    }

    Ok(())
}

fn apply_ruleset_changes(
    changes: &[SpecChange],
    repo: &str,
    client: &dyn GhRepoClient,
) -> anyhow::Result<()> {
    let org = org_from_repo(repo);
    for change in changes {
        match change {
            SpecChange::RulesetAdd { spec: rs, .. } => {
                tracing::info!("creating ruleset: {}", rs.name);
                let body = spec_ruleset_to_api_body(rs, org, client)?;
                client.create_ruleset(repo, &body)?;
            }
            SpecChange::RulesetUpdate { id, spec: rs, .. } => {
                tracing::info!("updating ruleset: {} (id={id})", rs.name);
                let body = spec_ruleset_to_api_body(rs, org, client)?;
                client.update_ruleset(repo, *id, &body)?;
            }
            SpecChange::RulesetDelete { id, name } => {
                tracing::info!("deleting ruleset: {name} (id={id})");
                client.delete_ruleset(repo, *id)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn apply_branch_protection_changes(
    changes: &[SpecChange],
    repo: &str,
    client: &dyn GhRepoClient,
) -> anyhow::Result<()> {
    for change in changes {
        match change {
            SpecChange::BranchProtectionAdd { spec: bp } => {
                tracing::info!("creating branch protection: {}", bp.pattern);
                let body = spec_protection_to_api_body(bp);
                client.put_branch_protection(repo, &bp.pattern, &body)?;
            }
            SpecChange::BranchProtectionUpdate { spec: bp } => {
                tracing::info!("updating branch protection: {}", bp.pattern);
                let body = spec_protection_to_api_body(bp);
                client.put_branch_protection(repo, &bp.pattern, &body)?;
            }
            SpecChange::BranchProtectionRemove { pattern } => {
                tracing::info!("deleting branch protection: {pattern}");
                client.delete_branch_protection(repo, pattern)?;
            }
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn org_from_repo(repo: &str) -> &str {
    repo.split_once('/').map_or(repo, |(org, _)| org)
}

fn spec_ruleset_to_api_body(
    rs: &Ruleset,
    org: &str,
    client: &dyn GhRepoClient,
) -> anyhow::Result<serde_json::Value> {
    let mut body = serde_json::Map::new();
    body.insert(String::from("name"), serde_json::json!(rs.name));
    body.insert(
        String::from("target"),
        serde_json::json!(rs.target.as_deref().unwrap_or("branch")),
    );
    body.insert(
        String::from("enforcement"),
        serde_json::json!(rs.enforcement.as_deref().unwrap_or("active")),
    );

    if let Some(actors) = &rs.bypass_actors {
        let mut api_actors = Vec::new();
        for actor in actors {
            api_actors.push(resolve_bypass_actor(actor, org, client)?);
        }
        body.insert(String::from("bypass_actors"), serde_json::json!(api_actors));
    }

    if let Some(conds) = &rs.conditions {
        let mut conds_body = serde_json::Map::new();
        if let Some(ref_name) = &conds.ref_name {
            let mut rn = serde_json::Map::new();
            rn.insert(
                String::from("include"),
                serde_json::json!(ref_name.include.clone().unwrap_or_default()),
            );
            rn.insert(
                String::from("exclude"),
                serde_json::json!(ref_name.exclude.clone().unwrap_or_default()),
            );
            conds_body.insert(String::from("ref_name"), serde_json::Value::Object(rn));
        }
        body.insert(
            String::from("conditions"),
            serde_json::Value::Object(conds_body),
        );
    }

    if let Some(rules) = &rs.rules {
        body.insert(String::from("rules"), spec_rules_to_api(rules, client)?);
    }

    Ok(serde_json::Value::Object(body))
}

fn spec_rules_to_api(
    rules: &RulesetRules,
    client: &dyn GhRepoClient,
) -> anyhow::Result<serde_json::Value> {
    let mut api_rules: Vec<serde_json::Value> = Vec::new();

    if rules.non_fast_forward == Some(true) {
        api_rules.push(serde_json::json!({ "type": "non_fast_forward" }));
    }
    if rules.deletion == Some(true) {
        api_rules.push(serde_json::json!({ "type": "deletion" }));
    }
    if rules.creation == Some(true) {
        api_rules.push(serde_json::json!({ "type": "creation" }));
    }
    if rules.required_linear_history == Some(true) {
        api_rules.push(serde_json::json!({ "type": "required_linear_history" }));
    }
    if rules.required_signatures == Some(true) {
        api_rules.push(serde_json::json!({ "type": "required_signatures" }));
    }
    if let Some(pr) = &rules.pull_request {
        let count = pr.required_approving_review_count.unwrap_or(1);
        let dismiss_stale = pr.dismiss_stale_reviews_on_push.unwrap_or(false);
        let require_code_owner = pr.require_code_owner_review.unwrap_or(false);
        let require_last_push = pr.require_last_push_approval.unwrap_or(false);
        let thread_resolution = pr.required_review_thread_resolution.unwrap_or(false);
        let mut pr_params = serde_json::json!({
            "required_approving_review_count": count,
            "dismiss_stale_reviews_on_push": dismiss_stale,
            "require_code_owner_review": require_code_owner,
            "require_last_push_approval": require_last_push,
            "required_review_thread_resolution": thread_resolution
        });
        if let Some(methods) = &pr.allowed_merge_methods
            && let Some(obj) = pr_params.as_object_mut()
        {
            obj.insert(
                String::from("allowed_merge_methods"),
                serde_json::json!(methods),
            );
        }
        api_rules.push(serde_json::json!({
            "type": "pull_request",
            "parameters": pr_params
        }));
    }
    if let Some(sc) = &rules.required_status_checks {
        let strict = sc.strict_required_status_checks_policy.unwrap_or(false);
        let mut contexts: Vec<serde_json::Value> = Vec::new();
        for c in sc.contexts.as_deref().unwrap_or(&[]) {
            // Determine the integration_id to attach.
            // Priority: explicit integration_id > app slug > default "github-actions".
            // Using an explicit integration_id skips the API lookup entirely.
            let id = if let Some(id) = c.integration_id {
                id
            } else {
                let slug = c.app.as_deref().unwrap_or("github-actions");
                client.resolve_app_id(slug)?
            };
            let mut ctx = serde_json::Map::new();
            ctx.insert(String::from("context"), serde_json::json!(c.context));
            ctx.insert(String::from("integration_id"), serde_json::json!(id));
            contexts.push(serde_json::Value::Object(ctx));
        }
        api_rules.push(serde_json::json!({
            "type": "required_status_checks",
            "parameters": {
                "strict_required_status_checks_policy": strict,
                "required_status_checks": contexts
            }
        }));
    }

    Ok(serde_json::json!(api_rules))
}

pub(super) fn spec_protection_to_api_body(bp: &BranchProtection) -> serde_json::Value {
    let required_status_checks = bp.require_status_checks.as_ref().map(|sc| {
        serde_json::json!({
            "strict": sc.strict.unwrap_or(false),
            "contexts": sc.contexts.clone().unwrap_or_default()
        })
    });

    let has_review = bp.required_reviews.is_some()
        || bp.dismiss_stale_reviews.is_some()
        || bp.require_code_owner_reviews.is_some();
    let required_pull_request_reviews = if has_review {
        Some(serde_json::json!({
            "dismiss_stale_reviews": bp.dismiss_stale_reviews.unwrap_or(false),
            "require_code_owner_reviews": bp.require_code_owner_reviews.unwrap_or(false),
            "required_approving_review_count": bp.required_reviews.unwrap_or(1)
        }))
    } else {
        None
    };

    serde_json::json!({
        "required_status_checks": required_status_checks,
        "enforce_admins": bp.enforce_admins.unwrap_or(false),
        "required_pull_request_reviews": required_pull_request_reviews,
        "restrictions": serde_json::Value::Null,
        "allow_force_pushes": bp.allow_force_pushes.unwrap_or(false),
        "allow_deletions": bp.allow_deletions.unwrap_or(false)
    })
}

fn resolve_bypass_actor(
    actor: &BypassActor,
    org: &str,
    client: &dyn GhRepoClient,
) -> anyhow::Result<serde_json::Value> {
    let bypass_mode = actor.bypass_mode.as_deref().unwrap_or("always");

    if let Some(role) = &actor.role {
        let id =
            role_actor_id(role).ok_or_else(|| anyhow::anyhow!("unknown bypass role: {role}"))?;
        return Ok(serde_json::json!({
            "actor_id": id,
            "actor_type": "RepositoryRole",
            "bypass_mode": bypass_mode
        }));
    }

    if actor.org_admin == Some(true) {
        return Ok(serde_json::json!({
            "actor_id": 1,
            "actor_type": "OrganizationAdmin",
            "bypass_mode": bypass_mode
        }));
    }

    if let Some(team_slug) = &actor.team {
        let id = client.resolve_team_id(org, team_slug)?;
        return Ok(serde_json::json!({
            "actor_id": id,
            "actor_type": "Team",
            "bypass_mode": bypass_mode
        }));
    }

    if let Some(app_slug) = &actor.app {
        let id = client.resolve_app_id(app_slug)?;
        return Ok(serde_json::json!({
            "actor_id": id,
            "actor_type": "Integration",
            "bypass_mode": bypass_mode
        }));
    }

    if let Some(role_name) = &actor.custom_role {
        let id = client.resolve_org_custom_role_id(org, role_name)?;
        return Ok(serde_json::json!({
            "actor_id": id,
            "actor_type": "RepositoryRole",
            "bypass_mode": bypass_mode
        }));
    }

    anyhow::bail!("bypass actor has no recognized type set")
}

/// Well-known GitHub `RepositoryRole` actor IDs.
pub(super) fn role_actor_id(role: &str) -> Option<u64> {
    match role {
        "read" => Some(1),
        "triage" => Some(3),
        "write" => Some(2),
        "maintain" => Some(4),
        "admin" => Some(5),
        _ => None,
    }
}
