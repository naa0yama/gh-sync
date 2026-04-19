#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]
#![allow(clippy::indexing_slicing)]
#![allow(missing_docs)]
#![allow(clippy::missing_docs_in_private_items)]
#![allow(clippy::wildcard_imports)]

use std::process::ExitCode;

use gh_sync_manifest::{
    Actions, BranchProtection, BranchProtectionStatusChecks, Features, Label, MergeStrategy,
    PullRequestRule, RefNameCondition, RequiredStatusChecks, Ruleset, RulesetConditions,
    RulesetRules, SelectedActions, Spec, StatusCheckContext,
};

use super::apply::{
    core_field_to_patch, org_from_repo, role_actor_id, spec_protection_to_api_body,
};
use super::compare::{
    compare_actions, compare_features, compare_merge_strategy, compare_topics, normalize_color,
    protection_matches, push_bool_field, push_str_field, ruleset_conditions_match, ruleset_matches,
    ruleset_pr_matches, ruleset_rules_match, ruleset_status_checks_match,
};
use super::testing::MockRepoClient;
use super::*;

fn make_spec() -> Spec {
    Spec {
        description: None,
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
    }
}

// ------------------------------------------------------------------
// compare: field-level
// ------------------------------------------------------------------

#[test]
fn description_changed() {
    let client = MockRepoClient::new("owner/repo");
    let spec = Spec {
        description: Some(String::from("new description")),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldChanged { field, new, .. }
        if field == "description" && new == "new description"
    )));
}

#[test]
fn description_unchanged() {
    let mut client = MockRepoClient::new("owner/repo");
    client.repo_data.description = Some(String::from("same"));
    let spec = Spec {
        description: Some(String::from("same")),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(
        changes
            .iter()
            .any(|c| matches!(c, SpecChange::FieldOk { field, .. } if field == "description"))
    );
}

#[test]
fn topics_changed() {
    let mut client = MockRepoClient::new("owner/repo");
    client.topics = vec![String::from("old")];
    let spec = Spec {
        topics: Some(vec![String::from("rust"), String::from("cli")]),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(
        changes
            .iter()
            .any(|c| matches!(c, SpecChange::FieldChanged { field, .. } if field == "topics"))
    );
}

#[test]
fn topics_unchanged() {
    let mut client = MockRepoClient::new("owner/repo");
    client.topics = vec![String::from("rust")];
    let spec = Spec {
        topics: Some(vec![String::from("rust")]),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(
        changes
            .iter()
            .any(|c| matches!(c, SpecChange::FieldOk { field, .. } if field == "topics"))
    );
}

// ------------------------------------------------------------------
// compare: features
// ------------------------------------------------------------------

#[test]
fn features_wiki_disable() {
    let mut client = MockRepoClient::new("owner/repo");
    client.repo_data.has_wiki = true;
    let spec = Spec {
        features: Some(Features {
            wiki: Some(false),
            issues: None,
            projects: None,
            discussions: None,
        }),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldChanged { field, new, .. }
        if field == "features.wiki" && new == "false"
    )));
}

// ------------------------------------------------------------------
// compare: labels
// ------------------------------------------------------------------

#[test]
fn label_add() {
    let client = MockRepoClient::new("owner/repo");
    let spec = Spec {
        labels: Some(vec![Label {
            name: String::from("kind/bug"),
            color: String::from("d73a4a"),
            description: Some(String::from("A bug")),
        }]),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(
        changes
            .iter()
            .any(|c| matches!(c, SpecChange::LabelAdd { name, .. } if name == "kind/bug"))
    );
}

#[test]
fn label_ok_when_matches() {
    let mut client = MockRepoClient::new("owner/repo");
    client.labels = vec![ApiLabel {
        name: String::from("kind/bug"),
        color: String::from("d73a4a"),
        description: Some(String::from("A bug")),
    }];
    let spec = Spec {
        labels: Some(vec![Label {
            name: String::from("kind/bug"),
            color: String::from("d73a4a"),
            description: Some(String::from("A bug")),
        }]),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(
        changes
            .iter()
            .any(|c| matches!(c, SpecChange::LabelOk { name } if name == "kind/bug"))
    );
}

#[test]
fn label_delete_in_mirror_mode() {
    let mut client = MockRepoClient::new("owner/repo");
    client.labels = vec![ApiLabel {
        name: String::from("unmanaged-label"),
        color: String::from("ffffff"),
        description: None,
    }];
    let spec = Spec {
        label_sync: Some(String::from("mirror")),
        labels: Some(vec![]),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::LabelDelete { name } if name == "unmanaged-label"
    )));
}

#[test]
fn label_not_deleted_in_additive_mode() {
    let mut client = MockRepoClient::new("owner/repo");
    client.labels = vec![ApiLabel {
        name: String::from("keep-me"),
        color: String::from("ffffff"),
        description: None,
    }];
    let spec = Spec {
        label_sync: Some(String::from("additive")),
        labels: Some(vec![]),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(
        !changes
            .iter()
            .any(|c| matches!(c, SpecChange::LabelDelete { .. }))
    );
}

// ------------------------------------------------------------------
// compare: workflow permissions
// ------------------------------------------------------------------

#[test]
fn workflow_permissions_changed() {
    use Actions;
    let mut client = MockRepoClient::new("owner/repo");
    client.workflow_permissions.default_workflow_permissions = Some(String::from("read"));
    let spec = Spec {
        actions: Some(Actions {
            enabled: None,
            allowed_actions: None,
            sha_pinning_required: None,
            workflow_permissions: Some(String::from("write")),
            can_approve_pull_requests: None,
            selected_actions: None,
            fork_pr_approval: None,
        }),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldChanged { field, new, .. }
        if field == "actions.workflow_permissions" && new == "write"
    )));
}

#[test]
fn workflow_permissions_unchanged() {
    use Actions;
    let mut client = MockRepoClient::new("owner/repo");
    client.workflow_permissions.default_workflow_permissions = Some(String::from("read"));
    let spec = Spec {
        actions: Some(Actions {
            enabled: None,
            allowed_actions: None,
            sha_pinning_required: None,
            workflow_permissions: Some(String::from("read")),
            can_approve_pull_requests: None,
            selected_actions: None,
            fork_pr_approval: None,
        }),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldOk { field, .. }
        if field == "actions.workflow_permissions"
    )));
}

// ------------------------------------------------------------------
// compare: rulesets
// ------------------------------------------------------------------

#[test]
fn ruleset_add_when_not_in_live() {
    let client = MockRepoClient::new("owner/repo");
    let spec = Spec {
        rulesets: Some(vec![Ruleset {
            name: String::from("protect-main"),
            target: Some(String::from("branch")),
            enforcement: Some(String::from("active")),
            bypass_actors: None,
            conditions: Some(RulesetConditions {
                ref_name: Some(RefNameCondition {
                    include: Some(vec![String::from("refs/heads/main")]),
                    exclude: None,
                }),
            }),
            rules: Some(RulesetRules {
                non_fast_forward: Some(true),
                deletion: Some(true),
                creation: None,
                required_linear_history: None,
                required_signatures: None,
                pull_request: Some(PullRequestRule {
                    required_approving_review_count: Some(1),
                    dismiss_stale_reviews_on_push: None,
                    require_code_owner_review: None,
                    require_last_push_approval: None,
                    required_review_thread_resolution: None,
                    allowed_merge_methods: None,
                }),
                required_status_checks: None,
            }),
        }]),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::RulesetAdd { name, .. } if name == "protect-main"
    )));
}

#[test]
fn ruleset_update_when_exists_in_live() {
    let mut client = MockRepoClient::new("owner/repo");
    client.rulesets = vec![LiveRuleset {
        id: 42,
        name: String::from("protect-main"),
    }];
    let spec = Spec {
        rulesets: Some(vec![Ruleset {
            name: String::from("protect-main"),
            target: None,
            enforcement: None,
            bypass_actors: None,
            conditions: None,
            rules: None,
        }]),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::RulesetUpdate { id: 42, name, .. } if name == "protect-main"
    )));
}

#[test]
fn ruleset_delete_when_not_in_spec() {
    let mut client = MockRepoClient::new("owner/repo");
    client.rulesets = vec![LiveRuleset {
        id: 99,
        name: String::from("old-ruleset"),
    }];
    let spec = Spec {
        rulesets: Some(vec![]),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::RulesetDelete { id: 99, name, .. } if name == "old-ruleset"
    )));
}

// ------------------------------------------------------------------
// compare: branch protection
// ------------------------------------------------------------------

#[test]
fn branch_protection_add_when_no_live() {
    let client = MockRepoClient::new("owner/repo");
    let spec = Spec {
        branch_protection: Some(vec![BranchProtection {
            pattern: String::from("main"),
            required_reviews: Some(1),
            dismiss_stale_reviews: None,
            require_code_owner_reviews: None,
            require_status_checks: None,
            enforce_admins: None,
            allow_force_pushes: Some(false),
            allow_deletions: Some(false),
        }]),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::BranchProtectionAdd { spec } if spec.pattern == "main"
    )));
}

#[test]
fn branch_protection_ok_when_matches() {
    let mut client = MockRepoClient::new("owner/repo");
    client.branch_protections.insert(
        String::from("main"),
        Some(BranchProtectionApi {
            required_reviews: Some(1),
            dismiss_stale_reviews: false,
            require_code_owner_reviews: false,
            strict_status_checks: false,
            status_check_contexts: vec![],
            enforce_admins: false,
            allow_force_pushes: false,
            allow_deletions: false,
        }),
    );
    let spec = Spec {
        branch_protection: Some(vec![BranchProtection {
            pattern: String::from("main"),
            required_reviews: Some(1),
            dismiss_stale_reviews: None,
            require_code_owner_reviews: None,
            require_status_checks: None,
            enforce_admins: None,
            allow_force_pushes: None,
            allow_deletions: None,
        }]),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::BranchProtectionOk { pattern } if pattern == "main"
    )));
}

#[test]
fn branch_protection_update_when_differs() {
    let mut client = MockRepoClient::new("owner/repo");
    client.branch_protections.insert(
        String::from("main"),
        Some(BranchProtectionApi {
            required_reviews: Some(0),
            dismiss_stale_reviews: false,
            require_code_owner_reviews: false,
            strict_status_checks: false,
            status_check_contexts: vec![],
            enforce_admins: false,
            allow_force_pushes: false,
            allow_deletions: false,
        }),
    );
    let spec = Spec {
        branch_protection: Some(vec![BranchProtection {
            pattern: String::from("main"),
            required_reviews: Some(2),
            dismiss_stale_reviews: None,
            require_code_owner_reviews: None,
            require_status_checks: None,
            enforce_admins: None,
            allow_force_pushes: None,
            allow_deletions: None,
        }]),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::BranchProtectionUpdate { spec } if spec.pattern == "main"
    )));
}

#[test]
fn branch_protection_remove_when_not_in_spec() {
    let mut client = MockRepoClient::new("owner/repo");
    client.protected_branches = vec![String::from("old-branch")];
    let spec = Spec {
        branch_protection: Some(vec![]),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::BranchProtectionRemove { pattern } if pattern == "old-branch"
    )));
}

// ------------------------------------------------------------------
// apply_changes
// ------------------------------------------------------------------

#[test]
fn apply_creates_label() {
    let spec = Spec {
        labels: Some(vec![Label {
            name: String::from("kind/bug"),
            color: String::from("d73a4a"),
            description: None,
        }]),
        ..make_spec()
    };
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::LabelAdd {
        name: String::from("kind/bug"),
        color: String::from("d73a4a"),
        description: None,
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    assert_eq!(*client.created_labels.borrow(), vec!["kind/bug"]);
}

#[test]
fn apply_deletes_label() {
    let spec = make_spec();
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::LabelDelete {
        name: String::from("old-label"),
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    assert_eq!(*client.deleted_labels.borrow(), vec!["old-label"]);
}

#[test]
fn apply_creates_ruleset() {
    let rs = Ruleset {
        name: String::from("protect-main"),
        target: Some(String::from("branch")),
        enforcement: Some(String::from("active")),
        bypass_actors: None,
        conditions: None,
        rules: None,
    };
    let spec = Spec {
        rulesets: Some(vec![rs.clone()]),
        ..make_spec()
    };
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::RulesetAdd {
        name: String::from("protect-main"),
        spec: Box::new(rs),
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    assert_eq!(client.created_rulesets.borrow().len(), 1);
    assert_eq!(
        client.created_rulesets.borrow()[0]["name"]
            .as_str()
            .unwrap(),
        "protect-main"
    );
}

#[test]
fn apply_creates_ruleset_status_checks_resolves_app_id() {
    use gh_sync_manifest::{RequiredStatusChecks, RulesetRules, StatusCheckContext};
    let rs = Ruleset {
        name: String::from("protect-main"),
        target: None,
        enforcement: None,
        bypass_actors: None,
        conditions: None,
        rules: Some(RulesetRules {
            non_fast_forward: None,
            deletion: None,
            creation: None,
            required_linear_history: None,
            required_signatures: None,
            pull_request: None,
            required_status_checks: Some(RequiredStatusChecks {
                strict_required_status_checks_policy: Some(true),
                contexts: Some(vec![
                    // Explicit app slug — resolved via resolve_app_id.
                    StatusCheckContext {
                        context: String::from("ci/test"),
                        app: Some(String::from("my-app")),
                        integration_id: None,
                    },
                    // No app specified — defaults to "github-actions".
                    StatusCheckContext {
                        context: String::from("ci/lint"),
                        app: None,
                        integration_id: None,
                    },
                ]),
            }),
        }),
    };
    let spec = Spec {
        rulesets: Some(vec![rs.clone()]),
        ..make_spec()
    };
    let mut client = MockRepoClient::new("owner/repo");
    client.app_ids.insert(String::from("my-app"), 111);
    client.app_ids.insert(String::from("github-actions"), 15368);
    let changes = vec![SpecChange::RulesetAdd {
        name: String::from("protect-main"),
        spec: Box::new(rs),
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let created = client.created_rulesets.borrow();
    assert_eq!(created.len(), 1);
    let rules = &created[0]["rules"];
    let sc_rule = rules
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["type"] == "required_status_checks")
        .expect("required_status_checks rule not found");
    let checks = &sc_rule["parameters"]["required_status_checks"];
    // Explicit app slug: integration_id resolved via API.
    assert_eq!(checks[0]["context"], "ci/test");
    assert_eq!(checks[0]["integration_id"], 111);
    // No app specified: defaults to "github-actions", integration_id resolved.
    assert_eq!(checks[1]["context"], "ci/lint");
    assert_eq!(checks[1]["integration_id"], 15368);
}

#[test]
fn apply_creates_ruleset_status_checks_direct_integration_id() {
    use gh_sync_manifest::{RequiredStatusChecks, RulesetRules, StatusCheckContext};
    let rs = Ruleset {
        name: String::from("protect-main"),
        target: None,
        enforcement: None,
        bypass_actors: None,
        conditions: None,
        rules: Some(RulesetRules {
            non_fast_forward: None,
            deletion: None,
            creation: None,
            required_linear_history: None,
            required_signatures: None,
            pull_request: None,
            required_status_checks: Some(RequiredStatusChecks {
                strict_required_status_checks_policy: Some(false),
                contexts: Some(vec![
                    // Direct integration_id — no API call needed.
                    StatusCheckContext {
                        context: String::from("security/scan"),
                        app: None,
                        integration_id: Some(57789),
                    },
                ]),
            }),
        }),
    };
    let spec = Spec {
        rulesets: Some(vec![rs.clone()]),
        ..make_spec()
    };
    // No app_ids registered — API must not be called for direct IDs.
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::RulesetAdd {
        name: String::from("protect-main"),
        spec: Box::new(rs),
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let created = client.created_rulesets.borrow();
    let sc_rule = created[0]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["type"] == "required_status_checks")
        .expect("required_status_checks rule not found");
    let checks = &sc_rule["parameters"]["required_status_checks"];
    assert_eq!(checks[0]["context"], "security/scan");
    assert_eq!(checks[0]["integration_id"], 57789);
}

#[test]
fn apply_updates_ruleset() {
    let rs = Ruleset {
        name: String::from("protect-main"),
        target: None,
        enforcement: Some(String::from("disabled")),
        bypass_actors: None,
        conditions: None,
        rules: None,
    };
    let spec = Spec {
        rulesets: Some(vec![rs.clone()]),
        ..make_spec()
    };
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::RulesetUpdate {
        id: 77,
        name: String::from("protect-main"),
        spec: Box::new(rs),
        live: None,
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let updated = client.updated_rulesets.borrow();
    assert_eq!(updated.len(), 1);
    assert_eq!(updated[0].0, 77);
}

#[test]
fn apply_deletes_ruleset() {
    let spec = make_spec();
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::RulesetDelete {
        id: 55,
        name: String::from("old"),
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    assert_eq!(*client.deleted_rulesets.borrow(), vec![55u64]);
}

#[test]
fn apply_puts_branch_protection() {
    let bp = BranchProtection {
        pattern: String::from("main"),
        required_reviews: Some(1),
        dismiss_stale_reviews: None,
        require_code_owner_reviews: None,
        require_status_checks: None,
        enforce_admins: Some(true),
        allow_force_pushes: Some(false),
        allow_deletions: Some(false),
    };
    let spec = Spec {
        branch_protection: Some(vec![bp.clone()]),
        ..make_spec()
    };
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::BranchProtectionAdd { spec: Box::new(bp) }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let put = client.put_branch_protections.borrow();
    assert_eq!(put.len(), 1);
    assert_eq!(put[0].0, "main");
    assert_eq!(put[0].1["enforce_admins"], true);
}

#[test]
fn apply_workflow_permissions() {
    use Actions;
    let spec = Spec {
        actions: Some(Actions {
            enabled: None,
            allowed_actions: None,
            sha_pinning_required: None,
            workflow_permissions: Some(String::from("write")),
            can_approve_pull_requests: Some(true),
            selected_actions: None,
            fork_pr_approval: None,
        }),
        ..make_spec()
    };
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![
        SpecChange::FieldChanged {
            field: String::from("actions.workflow_permissions"),
            old: String::from("read"),
            new: String::from("write"),
        },
        SpecChange::FieldChanged {
            field: String::from("actions.can_approve_pull_requests"),
            old: String::from("false"),
            new: String::from("true"),
        },
    ];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let wf = client.put_workflow_permissions_body.borrow();
    assert!(wf.is_some());
    assert_eq!(
        wf.as_ref().unwrap()["default_workflow_permissions"],
        "write"
    );
    assert_eq!(
        wf.as_ref().unwrap()["can_approve_pull_request_reviews"],
        true
    );
}

// ------------------------------------------------------------------
// print_preview
// ------------------------------------------------------------------

#[test]
fn print_preview_shows_changed_and_ok() {
    let changes = vec![
        SpecChange::FieldChanged {
            field: String::from("description"),
            old: String::from("old"),
            new: String::from("new"),
        },
        SpecChange::FieldOk {
            field: String::from("visibility"),
            value: String::from("public"),
        },
    ];
    let mut buf: Vec<u8> = Vec::new();
    let (code, has_actions) = print_preview(&mut buf, &changes, "owner/repo").unwrap();
    let out = String::from_utf8(buf).unwrap();
    assert_eq!(code, ExitCode::SUCCESS);
    assert!(has_actions);
    assert!(out.contains("[CHANGED]"), "missing CHANGED tag: {out}");
    assert!(out.contains("[OK     ]"), "missing OK tag: {out}");
    assert!(out.contains("1 changed"), "missing summary: {out}");
}

#[test]
fn print_preview_all_ok() {
    let changes = vec![SpecChange::FieldOk {
        field: String::from("visibility"),
        value: String::from("public"),
    }];
    let mut buf: Vec<u8> = Vec::new();
    let (_, has_actions) = print_preview(&mut buf, &changes, "owner/repo").unwrap();
    assert!(!has_actions);
    let out = String::from_utf8(buf).unwrap();
    assert!(out.contains("all settings up to date"), "unexpected: {out}");
}

#[test]
fn print_preview_shows_ruleset_changes() {
    // Disable ANSI colour output so assertions match plain-text tags
    // regardless of whether the test runner enables colours (e.g. llvm-cov).
    console::set_colors_enabled(false);
    let rs = Ruleset {
        name: String::from("protect-main"),
        target: None,
        enforcement: None,
        bypass_actors: None,
        conditions: None,
        rules: None,
    };
    let changes = vec![
        SpecChange::RulesetAdd {
            name: String::from("new-ruleset"),
            spec: Box::new(rs.clone()),
        },
        SpecChange::RulesetUpdate {
            id: 1,
            name: String::from("protect-main"),
            spec: Box::new(rs),
            live: None,
        },
        SpecChange::RulesetDelete {
            id: 2,
            name: String::from("old-ruleset"),
        },
    ];
    let mut buf: Vec<u8> = Vec::new();
    let (_, has_actions) = print_preview(&mut buf, &changes, "owner/repo").unwrap();
    assert!(has_actions);
    let out = String::from_utf8(buf).unwrap();
    assert!(out.contains("[ADD    ]  rulesets/new-ruleset"));
    assert!(out.contains("[CHANGED]  rulesets/protect-main"));
    assert!(out.contains("[DELETE ]  rulesets/old-ruleset"));
}

// ------------------------------------------------------------------
// normalize_color
// ------------------------------------------------------------------

#[test]
fn normalize_color_strips_hash() {
    assert_eq!(normalize_color("#d73a4a"), "d73a4a");
}

#[test]
fn normalize_color_no_hash_passthrough() {
    assert_eq!(normalize_color("d73a4a"), "d73a4a");
}

#[test]
fn normalize_color_lowercases() {
    assert_eq!(normalize_color("#D73A4A"), "d73a4a");
}

// ------------------------------------------------------------------
// org_from_repo
// ------------------------------------------------------------------

#[test]
fn org_from_repo_with_slash() {
    assert_eq!(org_from_repo("myorg/myrepo"), "myorg");
}

#[test]
fn org_from_repo_without_slash() {
    assert_eq!(org_from_repo("standalone"), "standalone");
}

// ------------------------------------------------------------------
// role_actor_id
// ------------------------------------------------------------------

#[test]
fn role_actor_id_known_roles() {
    assert_eq!(role_actor_id("read"), Some(1));
    assert_eq!(role_actor_id("triage"), Some(3));
    assert_eq!(role_actor_id("write"), Some(2));
    assert_eq!(role_actor_id("maintain"), Some(4));
    assert_eq!(role_actor_id("admin"), Some(5));
}

#[test]
fn role_actor_id_unknown_returns_none() {
    assert_eq!(role_actor_id("owner"), None);
    assert_eq!(role_actor_id(""), None);
}

// ------------------------------------------------------------------
// push_str_field
// ------------------------------------------------------------------

#[test]
fn push_str_field_match_produces_ok() {
    let mut changes = Vec::new();
    push_str_field(&mut changes, "description", Some("hello"), "hello");
    assert!(matches!(
        &changes[0],
        SpecChange::FieldOk { field, value } if field == "description" && value == "hello"
    ));
}

#[test]
fn push_str_field_mismatch_produces_changed() {
    let mut changes = Vec::new();
    push_str_field(&mut changes, "description", Some("old"), "new");
    assert!(matches!(
        &changes[0],
        SpecChange::FieldChanged { field, old, new }
        if field == "description" && old == "old" && new == "new"
    ));
}

#[test]
fn push_str_field_none_live_treated_as_empty() {
    let mut changes = Vec::new();
    push_str_field(&mut changes, "homepage", None, "");
    assert!(matches!(
        &changes[0],
        SpecChange::FieldOk { field, .. } if field == "homepage"
    ));
}

#[test]
fn push_str_field_none_live_differs_from_spec() {
    let mut changes = Vec::new();
    push_str_field(&mut changes, "homepage", None, "https://example.com");
    assert!(matches!(
        &changes[0],
        SpecChange::FieldChanged { field, old, .. }
        if field == "homepage" && old.is_empty()
    ));
}

// ------------------------------------------------------------------
// push_bool_field
// ------------------------------------------------------------------

#[test]
fn push_bool_field_match_produces_ok() {
    let mut changes = Vec::new();
    push_bool_field(&mut changes, "archived", false, false);
    assert!(matches!(
        &changes[0],
        SpecChange::FieldOk { field, value } if field == "archived" && value == "false"
    ));
}

#[test]
fn push_bool_field_mismatch_produces_changed() {
    let mut changes = Vec::new();
    push_bool_field(&mut changes, "archived", false, true);
    assert!(matches!(
        &changes[0],
        SpecChange::FieldChanged { field, old, new }
        if field == "archived" && old == "false" && new == "true"
    ));
}

// ------------------------------------------------------------------
// compare_topics (direct)
// ------------------------------------------------------------------

#[test]
fn compare_topics_equal_sorted() {
    let mut changes = Vec::new();
    compare_topics(
        &mut changes,
        &[String::from("cli"), String::from("rust")],
        &[String::from("cli"), String::from("rust")],
    );
    assert!(matches!(&changes[0], SpecChange::FieldOk { field, .. } if field == "topics"));
}

#[test]
fn compare_topics_equal_unsorted_is_ok() {
    let mut changes = Vec::new();
    compare_topics(
        &mut changes,
        &[String::from("rust"), String::from("cli")],
        &[String::from("cli"), String::from("rust")],
    );
    assert!(matches!(&changes[0], SpecChange::FieldOk { field, .. } if field == "topics"));
}

#[test]
fn compare_topics_different_produces_changed() {
    let mut changes = Vec::new();
    compare_topics(&mut changes, &[String::from("rust")], &[String::from("go")]);
    assert!(matches!(
        &changes[0],
        SpecChange::FieldChanged { field, .. } if field == "topics"
    ));
}

// ------------------------------------------------------------------
// compare_features (direct)
// ------------------------------------------------------------------

#[test]
fn compare_features_all_match() {
    let live = RepoApiData {
        description: None,
        homepage: None,
        visibility: String::from("public"),
        archived: false,
        has_issues: true,
        has_projects: true,
        has_wiki: false,
        has_discussions: false,
        delete_branch_on_merge: false,
        allow_merge_commit: true,
        allow_squash_merge: true,
        allow_rebase_merge: true,
        allow_auto_merge: false,
        allow_update_branch: false,
        web_commit_signoff_required: false,
        squash_merge_commit_title: None,
        squash_merge_commit_message: None,
        merge_commit_title: None,
        merge_commit_message: None,
    };
    let feat = Features {
        issues: Some(true),
        projects: Some(true),
        wiki: Some(false),
        discussions: Some(false),
    };
    let mut changes = Vec::new();
    compare_features(&mut changes, &feat, &live);
    assert!(
        changes
            .iter()
            .all(|c| matches!(c, SpecChange::FieldOk { .. }))
    );
}

#[test]
fn compare_features_discussions_mismatch() {
    let mut live = MockRepoClient::new("o/r").repo_data;
    live.has_discussions = false;
    let feat = Features {
        issues: None,
        projects: None,
        wiki: None,
        discussions: Some(true),
    };
    let mut changes = Vec::new();
    compare_features(&mut changes, &feat, &live);
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldChanged { field, new, .. }
        if field == "features.discussions" && new == "true"
    )));
}

// ------------------------------------------------------------------
// compare_merge_strategy (direct)
// ------------------------------------------------------------------

#[test]
fn compare_merge_strategy_bool_changed() {
    let live = MockRepoClient::new("o/r").repo_data;
    // live.allow_auto_merge is false; spec sets true
    let ms = MergeStrategy {
        allow_merge_commit: None,
        allow_squash_merge: None,
        allow_rebase_merge: None,
        allow_auto_merge: Some(true),
        allow_update_branch: None,
        auto_delete_head_branches: None,
        merge_commit_title: None,
        merge_commit_message: None,
        squash_merge_commit_title: None,
        squash_merge_commit_message: None,
    };
    let mut changes = Vec::new();
    compare_merge_strategy(&mut changes, &ms, &live);
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldChanged { field, new, .. }
        if field == "merge_strategy.allow_auto_merge" && new == "true"
    )));
}

#[test]
fn compare_merge_strategy_string_fields() {
    let live = MockRepoClient::new("o/r").repo_data;
    let ms = MergeStrategy {
        allow_merge_commit: None,
        allow_squash_merge: None,
        allow_rebase_merge: None,
        allow_auto_merge: None,
        allow_update_branch: None,
        auto_delete_head_branches: None,
        merge_commit_title: Some(String::from("PR_TITLE")),
        merge_commit_message: Some(String::from("BLANK")),
        squash_merge_commit_title: Some(String::from("COMMIT_OR_PR_TITLE")),
        squash_merge_commit_message: Some(String::from("PR_BODY")),
    };
    let mut changes = Vec::new();
    compare_merge_strategy(&mut changes, &ms, &live);
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldChanged { field, .. }
        if field == "merge_strategy.merge_commit_title"
    )));
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldChanged { field, .. }
        if field == "merge_strategy.squash_merge_commit_title"
    )));
}

#[test]
fn compare_merge_strategy_all_ok() {
    let mut live = MockRepoClient::new("o/r").repo_data;
    live.squash_merge_commit_title = Some(String::from("PR_TITLE"));
    let ms = MergeStrategy {
        allow_merge_commit: Some(live.allow_merge_commit),
        allow_squash_merge: Some(live.allow_squash_merge),
        allow_rebase_merge: Some(live.allow_rebase_merge),
        allow_auto_merge: Some(live.allow_auto_merge),
        allow_update_branch: Some(live.allow_update_branch),
        auto_delete_head_branches: Some(live.delete_branch_on_merge),
        merge_commit_title: None,
        merge_commit_message: None,
        squash_merge_commit_title: Some(String::from("PR_TITLE")),
        squash_merge_commit_message: None,
    };
    let mut changes = Vec::new();
    compare_merge_strategy(&mut changes, &ms, &live);
    assert!(
        changes
            .iter()
            .all(|c| matches!(c, SpecChange::FieldOk { .. }))
    );
}

// ------------------------------------------------------------------
// compare_labels: label update case
// ------------------------------------------------------------------

#[test]
fn label_update_when_color_differs() {
    let mut client = MockRepoClient::new("owner/repo");
    client.labels = vec![ApiLabel {
        name: String::from("bug"),
        color: String::from("ff0000"),
        description: Some(String::from("A bug")),
    }];
    let spec = Spec {
        labels: Some(vec![Label {
            name: String::from("bug"),
            color: String::from("0000ff"),
            description: Some(String::from("A bug")),
        }]),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::LabelUpdate { name, new_color, .. }
        if name == "bug" && new_color == "0000ff"
    )));
}

#[test]
fn label_update_when_description_differs() {
    let mut client = MockRepoClient::new("owner/repo");
    client.labels = vec![ApiLabel {
        name: String::from("bug"),
        color: String::from("d73a4a"),
        description: Some(String::from("old desc")),
    }];
    let spec = Spec {
        labels: Some(vec![Label {
            name: String::from("bug"),
            color: String::from("d73a4a"),
            description: Some(String::from("new desc")),
        }]),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::LabelUpdate { name, new_description, .. }
        if name == "bug" && new_description.as_deref() == Some("new desc")
    )));
}

#[test]
fn label_ok_when_color_has_hash_prefix() {
    // Spec has "#d73a4a", live has "d73a4a" — normalize_color strips the #
    let mut client = MockRepoClient::new("owner/repo");
    client.labels = vec![ApiLabel {
        name: String::from("bug"),
        color: String::from("d73a4a"),
        description: None,
    }];
    let spec = Spec {
        labels: Some(vec![Label {
            name: String::from("bug"),
            color: String::from("#d73a4a"),
            description: None,
        }]),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(
        changes
            .iter()
            .any(|c| matches!(c, SpecChange::LabelOk { name } if name == "bug"))
    );
}

// ------------------------------------------------------------------
// compare_actions (direct)
// ------------------------------------------------------------------

#[test]
fn compare_actions_enabled_changed() {
    let live_act = ActionsPermissionsApi {
        enabled: true,
        allowed_actions: None,
        sha_pinning_required: None,
    };
    let act = Actions {
        enabled: Some(false),
        allowed_actions: None,
        sha_pinning_required: None,
        workflow_permissions: None,
        can_approve_pull_requests: None,
        selected_actions: None,
        fork_pr_approval: None,
    };
    let mut changes = Vec::new();
    compare_actions(&mut changes, &act, &live_act, None, None, None);
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldChanged { field, new, .. }
        if field == "actions.enabled" && new == "false"
    )));
}

#[test]
fn compare_actions_allowed_actions_unchanged() {
    let live_act = ActionsPermissionsApi {
        enabled: true,
        allowed_actions: Some(String::from("selected")),
        sha_pinning_required: None,
    };
    let act = Actions {
        enabled: None,
        allowed_actions: Some(String::from("selected")),
        sha_pinning_required: None,
        workflow_permissions: None,
        can_approve_pull_requests: None,
        selected_actions: None,
        fork_pr_approval: None,
    };
    let mut changes = Vec::new();
    compare_actions(&mut changes, &act, &live_act, None, None, None);
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldOk { field, .. } if field == "actions.allowed_actions"
    )));
}

#[test]
fn compare_actions_workflow_permissions_changed() {
    let live_act = ActionsPermissionsApi {
        enabled: true,
        allowed_actions: None,
        sha_pinning_required: None,
    };
    let live_wf = WorkflowPermissionsApi {
        default_workflow_permissions: Some(String::from("read")),
        can_approve_pull_request_reviews: false,
    };
    let act = Actions {
        enabled: None,
        allowed_actions: None,
        sha_pinning_required: None,
        workflow_permissions: Some(String::from("write")),
        can_approve_pull_requests: None,
        selected_actions: None,
        fork_pr_approval: None,
    };
    let mut changes = Vec::new();
    compare_actions(&mut changes, &act, &live_act, None, Some(&live_wf), None);
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldChanged { field, new, .. }
        if field == "actions.workflow_permissions" && new == "write"
    )));
}

#[test]
fn compare_actions_fork_pr_approval_skipped_when_no_live() {
    let live_act = ActionsPermissionsApi {
        enabled: true,
        allowed_actions: None,
        sha_pinning_required: None,
    };
    let act = Actions {
        enabled: None,
        allowed_actions: None,
        sha_pinning_required: None,
        workflow_permissions: None,
        can_approve_pull_requests: None,
        selected_actions: None,
        fork_pr_approval: Some(String::from("not_approved")),
    };
    let mut changes = Vec::new();
    compare_actions(&mut changes, &act, &live_act, None, None, None);
    // When live_fork_pr is None the field should be reported as Ok (skipped)
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldOk { field, .. } if field == "actions.fork_pr_approval"
    )));
}

#[test]
fn compare_actions_sha_pinning_changed() {
    let live_act = ActionsPermissionsApi {
        enabled: true,
        allowed_actions: None,
        sha_pinning_required: Some(false),
    };
    let act = Actions {
        enabled: None,
        allowed_actions: None,
        sha_pinning_required: Some(true),
        workflow_permissions: None,
        can_approve_pull_requests: None,
        selected_actions: None,
        fork_pr_approval: None,
    };
    let mut changes = Vec::new();
    compare_actions(&mut changes, &act, &live_act, None, None, None);
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldChanged { field, new, .. }
        if field == "actions.sha_pinning_required" && new == "true"
    )));
}

#[test]
fn compare_actions_selected_actions_patterns_ok() {
    let live_act = ActionsPermissionsApi {
        enabled: true,
        allowed_actions: Some(String::from("selected")),
        sha_pinning_required: None,
    };
    let live_sel = SelectedActionsApi {
        github_owned_allowed: Some(true),
        patterns_allowed: Some(vec![String::from("actions/*"), String::from("github/*")]),
    };
    let act = Actions {
        enabled: None,
        allowed_actions: None,
        sha_pinning_required: None,
        workflow_permissions: None,
        can_approve_pull_requests: None,
        selected_actions: Some(SelectedActions {
            github_owned_allowed: Some(true),
            patterns_allowed: Some(vec![String::from("github/*"), String::from("actions/*")]),
        }),
        fork_pr_approval: None,
    };
    let mut changes = Vec::new();
    compare_actions(&mut changes, &act, &live_act, Some(&live_sel), None, None);
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldOk { field, .. }
        if field == "actions.selected_actions.patterns_allowed"
    )));
}

// ------------------------------------------------------------------
// ruleset_matches
// ------------------------------------------------------------------

#[test]
fn ruleset_matches_target_mismatch() {
    let spec = Ruleset {
        name: String::from("r"),
        target: Some(String::from("tag")),
        enforcement: None,
        bypass_actors: None,
        conditions: None,
        rules: None,
    };
    let live = serde_json::json!({ "target": "branch", "enforcement": "active" });
    assert!(!ruleset_matches(&spec, &live));
}

#[test]
fn ruleset_matches_enforcement_mismatch() {
    let spec = Ruleset {
        name: String::from("r"),
        target: None,
        enforcement: Some(String::from("disabled")),
        bypass_actors: None,
        conditions: None,
        rules: None,
    };
    let live = serde_json::json!({ "target": "branch", "enforcement": "active" });
    assert!(!ruleset_matches(&spec, &live));
}

#[test]
fn ruleset_matches_defaults_match() {
    let spec = Ruleset {
        name: String::from("r"),
        target: None,      // default "branch"
        enforcement: None, // default "active"
        bypass_actors: None,
        conditions: None,
        rules: None,
    };
    let live = serde_json::json!({ "target": "branch", "enforcement": "active" });
    assert!(ruleset_matches(&spec, &live));
}

// ------------------------------------------------------------------
// ruleset_conditions_match
// ------------------------------------------------------------------

#[test]
fn ruleset_conditions_match_include_order_independent() {
    let ref_name = RefNameCondition {
        include: Some(vec![String::from("main"), String::from("dev")]),
        exclude: None,
    };
    let live = serde_json::json!({
        "conditions": {
            "ref_name": {
                "include": ["dev", "main"],
                "exclude": []
            }
        }
    });
    assert!(ruleset_conditions_match(&ref_name, &live));
}

#[test]
fn ruleset_conditions_mismatch_include() {
    let ref_name = RefNameCondition {
        include: Some(vec![String::from("main")]),
        exclude: None,
    };
    let live = serde_json::json!({
        "conditions": {
            "ref_name": {
                "include": ["feature"],
                "exclude": []
            }
        }
    });
    assert!(!ruleset_conditions_match(&ref_name, &live));
}

#[test]
fn ruleset_conditions_match_exclude() {
    let ref_name = RefNameCondition {
        include: None,
        exclude: Some(vec![String::from("release/*")]),
    };
    let live = serde_json::json!({
        "conditions": {
            "ref_name": {
                "include": [],
                "exclude": ["release/*"]
            }
        }
    });
    assert!(ruleset_conditions_match(&ref_name, &live));
}

// ------------------------------------------------------------------
// ruleset_rules_match
// ------------------------------------------------------------------

#[test]
fn ruleset_rules_match_boolean_rule_enabled() {
    let rules = RulesetRules {
        non_fast_forward: Some(true),
        deletion: None,
        creation: None,
        required_linear_history: None,
        required_signatures: None,
        pull_request: None,
        required_status_checks: None,
    };
    let live_rules = vec![serde_json::json!({ "type": "non_fast_forward" })];
    assert!(ruleset_rules_match(&rules, &live_rules));
}

#[test]
fn ruleset_rules_match_boolean_rule_disabled_missing_is_ok() {
    let rules = RulesetRules {
        non_fast_forward: Some(false),
        deletion: None,
        creation: None,
        required_linear_history: None,
        required_signatures: None,
        pull_request: None,
        required_status_checks: None,
    };
    // spec says false, live has no such rule → match
    let live_rules: Vec<serde_json::Value> = vec![];
    assert!(ruleset_rules_match(&rules, &live_rules));
}

#[test]
fn ruleset_rules_match_boolean_rule_mismatch() {
    let rules = RulesetRules {
        non_fast_forward: Some(true),
        deletion: None,
        creation: None,
        required_linear_history: None,
        required_signatures: None,
        pull_request: None,
        required_status_checks: None,
    };
    // spec says true but live has no non_fast_forward rule
    let live_rules: Vec<serde_json::Value> = vec![];
    assert!(!ruleset_rules_match(&rules, &live_rules));
}

// ------------------------------------------------------------------
// ruleset_pr_matches
// ------------------------------------------------------------------

#[test]
fn ruleset_pr_matches_no_live_params_returns_false() {
    let spec_pr = PullRequestRule {
        required_approving_review_count: Some(1),
        dismiss_stale_reviews_on_push: None,
        require_code_owner_review: None,
        require_last_push_approval: None,
        required_review_thread_resolution: None,
        allowed_merge_methods: None,
    };
    assert!(!ruleset_pr_matches(&spec_pr, None));
}

#[test]
fn ruleset_pr_matches_all_match() {
    let spec_pr = PullRequestRule {
        required_approving_review_count: Some(2),
        dismiss_stale_reviews_on_push: Some(true),
        require_code_owner_review: Some(false),
        require_last_push_approval: None,
        required_review_thread_resolution: None,
        allowed_merge_methods: None,
    };
    let live = serde_json::json!({
        "required_approving_review_count": 2,
        "dismiss_stale_reviews_on_push": true,
        "require_code_owner_review": false
    });
    assert!(ruleset_pr_matches(&spec_pr, Some(&live)));
}

#[test]
fn ruleset_pr_matches_count_mismatch() {
    let spec_pr = PullRequestRule {
        required_approving_review_count: Some(2),
        dismiss_stale_reviews_on_push: None,
        require_code_owner_review: None,
        require_last_push_approval: None,
        required_review_thread_resolution: None,
        allowed_merge_methods: None,
    };
    let live = serde_json::json!({
        "required_approving_review_count": 1
    });
    assert!(!ruleset_pr_matches(&spec_pr, Some(&live)));
}

#[test]
fn ruleset_pr_matches_merge_methods_order_independent() {
    let spec_pr = PullRequestRule {
        required_approving_review_count: None,
        dismiss_stale_reviews_on_push: None,
        require_code_owner_review: None,
        require_last_push_approval: None,
        required_review_thread_resolution: None,
        allowed_merge_methods: Some(vec![String::from("squash"), String::from("merge")]),
    };
    let live = serde_json::json!({
        "allowed_merge_methods": ["merge", "squash"]
    });
    assert!(ruleset_pr_matches(&spec_pr, Some(&live)));
}

// ------------------------------------------------------------------
// ruleset_status_checks_match
// ------------------------------------------------------------------

#[test]
fn ruleset_status_checks_match_no_live_returns_false() {
    let spec_sc = RequiredStatusChecks {
        strict_required_status_checks_policy: Some(true),
        contexts: None,
    };
    assert!(!ruleset_status_checks_match(&spec_sc, None));
}

#[test]
fn ruleset_status_checks_match_strict_mismatch() {
    let spec_sc = RequiredStatusChecks {
        strict_required_status_checks_policy: Some(true),
        contexts: None,
    };
    let live = serde_json::json!({
        "strict_required_status_checks_policy": false
    });
    assert!(!ruleset_status_checks_match(&spec_sc, Some(&live)));
}

#[test]
fn ruleset_status_checks_match_contexts_ok() {
    let spec_sc = RequiredStatusChecks {
        strict_required_status_checks_policy: None,
        contexts: Some(vec![
            StatusCheckContext {
                context: String::from("ci"),
                app: None,
                integration_id: None,
            },
            StatusCheckContext {
                context: String::from("lint"),
                app: None,
                integration_id: None,
            },
        ]),
    };
    let live = serde_json::json!({
        "required_status_checks": [
            { "context": "lint" },
            { "context": "ci" }
        ]
    });
    assert!(ruleset_status_checks_match(&spec_sc, Some(&live)));
}

#[test]
fn ruleset_status_checks_match_contexts_mismatch() {
    let spec_sc = RequiredStatusChecks {
        strict_required_status_checks_policy: None,
        contexts: Some(vec![StatusCheckContext {
            context: String::from("ci"),
            app: None,
            integration_id: None,
        }]),
    };
    let live = serde_json::json!({
        "required_status_checks": [
            { "context": "lint" }
        ]
    });
    assert!(!ruleset_status_checks_match(&spec_sc, Some(&live)));
}

// ------------------------------------------------------------------
// protection_matches
// ------------------------------------------------------------------

fn make_live_bp() -> BranchProtectionApi {
    BranchProtectionApi {
        required_reviews: Some(1),
        dismiss_stale_reviews: false,
        require_code_owner_reviews: false,
        strict_status_checks: false,
        status_check_contexts: vec![],
        enforce_admins: false,
        allow_force_pushes: false,
        allow_deletions: false,
    }
}

#[test]
fn protection_matches_all_defaults_match() {
    let spec = BranchProtection {
        pattern: String::from("main"),
        required_reviews: Some(1),
        dismiss_stale_reviews: Some(false),
        require_code_owner_reviews: Some(false),
        require_status_checks: None,
        enforce_admins: Some(false),
        allow_force_pushes: Some(false),
        allow_deletions: Some(false),
    };
    assert!(protection_matches(&spec, &make_live_bp()));
}

#[test]
fn protection_matches_review_count_mismatch() {
    let spec = BranchProtection {
        pattern: String::from("main"),
        required_reviews: Some(2),
        dismiss_stale_reviews: None,
        require_code_owner_reviews: None,
        require_status_checks: None,
        enforce_admins: None,
        allow_force_pushes: None,
        allow_deletions: None,
    };
    assert!(!protection_matches(&spec, &make_live_bp()));
}

#[test]
fn protection_matches_enforce_admins_mismatch() {
    let spec = BranchProtection {
        pattern: String::from("main"),
        required_reviews: None,
        dismiss_stale_reviews: None,
        require_code_owner_reviews: None,
        require_status_checks: None,
        enforce_admins: Some(true),
        allow_force_pushes: None,
        allow_deletions: None,
    };
    assert!(!protection_matches(&spec, &make_live_bp()));
}

#[test]
fn protection_matches_status_checks_strict_mismatch() {
    let spec = BranchProtection {
        pattern: String::from("main"),
        required_reviews: None,
        dismiss_stale_reviews: None,
        require_code_owner_reviews: None,
        require_status_checks: Some(BranchProtectionStatusChecks {
            strict: Some(true),
            contexts: None,
        }),
        enforce_admins: None,
        allow_force_pushes: None,
        allow_deletions: None,
    };
    assert!(!protection_matches(&spec, &make_live_bp()));
}

#[test]
fn protection_matches_status_contexts_order_independent() {
    let spec = BranchProtection {
        pattern: String::from("main"),
        required_reviews: None,
        dismiss_stale_reviews: None,
        require_code_owner_reviews: None,
        require_status_checks: Some(BranchProtectionStatusChecks {
            strict: None,
            contexts: Some(vec![String::from("ci"), String::from("lint")]),
        }),
        enforce_admins: None,
        allow_force_pushes: None,
        allow_deletions: None,
    };
    let mut live = make_live_bp();
    live.status_check_contexts = vec![String::from("lint"), String::from("ci")];
    assert!(protection_matches(&spec, &live));
}

// ------------------------------------------------------------------
// core_field_to_patch
// ------------------------------------------------------------------

#[test]
fn core_field_to_patch_known_string_field() {
    let result = core_field_to_patch("description", "hello");
    let (key, val) = result.unwrap();
    assert_eq!(key, "description");
    assert_eq!(val, serde_json::json!("hello"));
}

#[test]
fn core_field_to_patch_known_bool_field() {
    let result = core_field_to_patch("archived", "true");
    let (key, val) = result.unwrap();
    assert_eq!(key, "archived");
    assert_eq!(val, serde_json::json!(true));
}

#[test]
fn core_field_to_patch_features_issues() {
    let (key, val) = core_field_to_patch("features.issues", "false").unwrap();
    assert_eq!(key, "has_issues");
    assert_eq!(val, serde_json::json!(false));
}

#[test]
fn core_field_to_patch_merge_strategy_squash_title() {
    let (key, val) =
        core_field_to_patch("merge_strategy.squash_merge_commit_title", "PR_TITLE").unwrap();
    assert_eq!(key, "squash_merge_commit_title");
    assert_eq!(val, serde_json::json!("PR_TITLE"));
}

#[test]
fn core_field_to_patch_unknown_returns_none() {
    assert!(core_field_to_patch("unknown.field", "value").is_none());
}

#[test]
fn core_field_to_patch_delete_branch_on_merge() {
    let (key, val) =
        core_field_to_patch("merge_strategy.auto_delete_head_branches", "true").unwrap();
    assert_eq!(key, "delete_branch_on_merge");
    assert_eq!(val, serde_json::json!(true));
}

// ------------------------------------------------------------------
// spec_protection_to_api_body
// ------------------------------------------------------------------

#[test]
fn spec_protection_to_api_body_minimal() {
    let bp = BranchProtection {
        pattern: String::from("main"),
        required_reviews: None,
        dismiss_stale_reviews: None,
        require_code_owner_reviews: None,
        require_status_checks: None,
        enforce_admins: None,
        allow_force_pushes: None,
        allow_deletions: None,
    };
    let body = spec_protection_to_api_body(&bp);
    assert_eq!(body["enforce_admins"], false);
    assert_eq!(body["allow_force_pushes"], false);
    assert_eq!(body["allow_deletions"], false);
    assert!(body["required_pull_request_reviews"].is_null());
    assert!(body["required_status_checks"].is_null());
}

#[test]
fn spec_protection_to_api_body_with_reviews() {
    let bp = BranchProtection {
        pattern: String::from("main"),
        required_reviews: Some(2),
        dismiss_stale_reviews: Some(true),
        require_code_owner_reviews: Some(true),
        require_status_checks: None,
        enforce_admins: Some(true),
        allow_force_pushes: None,
        allow_deletions: None,
    };
    let body = spec_protection_to_api_body(&bp);
    let reviews = &body["required_pull_request_reviews"];
    assert_eq!(reviews["required_approving_review_count"], 2);
    assert_eq!(reviews["dismiss_stale_reviews"], true);
    assert_eq!(reviews["require_code_owner_reviews"], true);
    assert_eq!(body["enforce_admins"], true);
}

#[test]
fn spec_protection_to_api_body_with_status_checks() {
    let bp = BranchProtection {
        pattern: String::from("main"),
        required_reviews: None,
        dismiss_stale_reviews: None,
        require_code_owner_reviews: None,
        require_status_checks: Some(BranchProtectionStatusChecks {
            strict: Some(true),
            contexts: Some(vec![String::from("ci/test")]),
        }),
        enforce_admins: None,
        allow_force_pushes: None,
        allow_deletions: None,
    };
    let body = spec_protection_to_api_body(&bp);
    let sc = &body["required_status_checks"];
    assert_eq!(sc["strict"], true);
    assert_eq!(sc["contexts"][0], "ci/test");
}

// ------------------------------------------------------------------
// parse_repo_api_data
// ------------------------------------------------------------------

#[test]
fn parse_repo_api_data_full_json() {
    let v = serde_json::json!({
        "description": "my repo",
        "homepage": "https://example.com",
        "visibility": "private",
        "archived": true,
        "has_issues": false,
        "has_projects": false,
        "has_wiki": false,
        "has_discussions": true,
        "delete_branch_on_merge": true,
        "allow_merge_commit": false,
        "allow_squash_merge": false,
        "allow_rebase_merge": false,
        "allow_auto_merge": true,
        "allow_update_branch": true,
        "web_commit_signoff_required": true,
        "squash_merge_commit_title": "PR_TITLE",
        "squash_merge_commit_message": "BLANK",
        "merge_commit_title": "MERGE_MESSAGE",
        "merge_commit_message": "PR_BODY"
    });
    let d = parse_repo_api_data(&v);
    assert_eq!(d.description, Some(String::from("my repo")));
    assert_eq!(d.homepage, Some(String::from("https://example.com")));
    assert_eq!(d.visibility, "private");
    assert!(d.archived);
    assert!(!d.has_issues);
    assert!(!d.has_wiki);
    assert!(d.has_discussions);
    assert!(d.delete_branch_on_merge);
    assert!(!d.allow_merge_commit);
    assert!(d.allow_auto_merge);
    assert!(d.web_commit_signoff_required);
    assert_eq!(d.squash_merge_commit_title, Some(String::from("PR_TITLE")));
    assert_eq!(d.merge_commit_message, Some(String::from("PR_BODY")));
}

#[test]
fn parse_repo_api_data_defaults_for_missing_fields() {
    let v = serde_json::json!({});
    let d = parse_repo_api_data(&v);
    assert_eq!(d.description, None);
    assert_eq!(d.visibility, "public");
    assert!(!d.archived);
    assert!(d.has_issues);
    assert!(d.has_wiki);
    assert!(!d.has_discussions);
    assert!(!d.allow_auto_merge);
}

#[test]
fn parse_repo_api_data_empty_description_treated_as_none() {
    let v = serde_json::json!({ "description": "" });
    let d = parse_repo_api_data(&v);
    assert_eq!(d.description, None);
}

// ------------------------------------------------------------------
// parse_branch_protection_api
// ------------------------------------------------------------------

#[test]
fn parse_branch_protection_api_full_json() {
    let v = serde_json::json!({
        "required_pull_request_reviews": {
            "required_approving_review_count": 2,
            "dismiss_stale_reviews": true,
            "require_code_owner_reviews": true
        },
        "required_status_checks": {
            "strict": true,
            "contexts": ["ci", "lint"]
        },
        "enforce_admins": { "enabled": true },
        "allow_force_pushes": { "enabled": false },
        "allow_deletions": { "enabled": true }
    });
    let bp = parse_branch_protection_api(&v);
    assert_eq!(bp.required_reviews, Some(2));
    assert!(bp.dismiss_stale_reviews);
    assert!(bp.require_code_owner_reviews);
    assert!(bp.strict_status_checks);
    assert_eq!(bp.status_check_contexts, vec!["ci", "lint"]);
    assert!(bp.enforce_admins);
    assert!(!bp.allow_force_pushes);
    assert!(bp.allow_deletions);
}

#[test]
fn parse_branch_protection_api_defaults_for_missing_fields() {
    let v = serde_json::json!({});
    let bp = parse_branch_protection_api(&v);
    assert_eq!(bp.required_reviews, None);
    assert!(!bp.dismiss_stale_reviews);
    assert!(!bp.strict_status_checks);
    assert!(bp.status_check_contexts.is_empty());
    assert!(!bp.enforce_admins);
}

// ------------------------------------------------------------------
// print_preview: label changes
// ------------------------------------------------------------------

#[test]
fn print_preview_label_add() {
    console::set_colors_enabled(false);
    let changes = vec![SpecChange::LabelAdd {
        name: String::from("kind/bug"),
        color: String::from("d73a4a"),
        description: Some(String::from("A bug")),
    }];
    let mut buf: Vec<u8> = Vec::new();
    let (_, has_actions) = print_preview(&mut buf, &changes, "owner/repo").unwrap();
    assert!(has_actions);
    let out = String::from_utf8(buf).unwrap();
    assert!(
        out.contains("--- labels ---"),
        "missing labels section: {out}"
    );
    assert!(
        out.contains("[ADD    ]  kind/bug"),
        "missing label add: {out}"
    );
}

#[test]
fn print_preview_label_update() {
    console::set_colors_enabled(false);
    let changes = vec![SpecChange::LabelUpdate {
        name: String::from("bug"),
        old_color: String::from("ff0000"),
        old_description: Some(String::from("old")),
        new_color: String::from("0000ff"),
        new_description: Some(String::from("new")),
    }];
    let mut buf: Vec<u8> = Vec::new();
    let (_, has_actions) = print_preview(&mut buf, &changes, "owner/repo").unwrap();
    assert!(has_actions);
    let out = String::from_utf8(buf).unwrap();
    assert!(
        out.contains("[CHANGED]  bug:"),
        "missing label update: {out}"
    );
}

#[test]
fn print_preview_label_delete() {
    console::set_colors_enabled(false);
    let changes = vec![SpecChange::LabelDelete {
        name: String::from("wontfix"),
    }];
    let mut buf: Vec<u8> = Vec::new();
    let (_, has_actions) = print_preview(&mut buf, &changes, "owner/repo").unwrap();
    assert!(has_actions);
    let out = String::from_utf8(buf).unwrap();
    assert!(
        out.contains("[DELETE ]  wontfix"),
        "missing label delete: {out}"
    );
}

#[test]
fn print_preview_label_ok() {
    console::set_colors_enabled(false);
    let changes = vec![SpecChange::LabelOk {
        name: String::from("good-label"),
    }];
    let mut buf: Vec<u8> = Vec::new();
    let (_, has_actions) = print_preview(&mut buf, &changes, "owner/repo").unwrap();
    assert!(!has_actions);
    let out = String::from_utf8(buf).unwrap();
    assert!(
        out.contains("[OK     ]  good-label"),
        "missing label ok: {out}"
    );
}

// ------------------------------------------------------------------
// print_preview: branch protection changes
// ------------------------------------------------------------------

#[test]
fn print_preview_branch_protection_changes() {
    console::set_colors_enabled(false);
    let bp = BranchProtection {
        pattern: String::from("main"),
        required_reviews: None,
        dismiss_stale_reviews: None,
        require_code_owner_reviews: None,
        require_status_checks: None,
        enforce_admins: None,
        allow_force_pushes: None,
        allow_deletions: None,
    };
    let changes = vec![
        SpecChange::BranchProtectionAdd {
            spec: Box::new(bp.clone()),
        },
        SpecChange::BranchProtectionUpdate { spec: Box::new(bp) },
        SpecChange::BranchProtectionRemove {
            pattern: String::from("old-branch"),
        },
        SpecChange::BranchProtectionOk {
            pattern: String::from("stable"),
        },
    ];
    let mut buf: Vec<u8> = Vec::new();
    let (_, has_actions) = print_preview(&mut buf, &changes, "owner/repo").unwrap();
    assert!(has_actions);
    let out = String::from_utf8(buf).unwrap();
    assert!(
        out.contains("[ADD    ]  branch_protection/main"),
        "missing BP add: {out}"
    );
    assert!(
        out.contains("[CHANGED]  branch_protection/main"),
        "missing BP update: {out}"
    );
    assert!(
        out.contains("[DELETE ]  branch_protection/old-branch"),
        "missing BP delete: {out}"
    );
    assert!(
        out.contains("[OK     ]  branch_protection/stable"),
        "missing BP ok: {out}"
    );
}

// ------------------------------------------------------------------
// print_preview: RulesetOk
// ------------------------------------------------------------------

#[test]
fn print_preview_ruleset_ok() {
    console::set_colors_enabled(false);
    let changes = vec![SpecChange::RulesetOk {
        id: 42,
        name: String::from("my-ruleset"),
    }];
    let mut buf: Vec<u8> = Vec::new();
    let (_, has_actions) = print_preview(&mut buf, &changes, "owner/repo").unwrap();
    assert!(!has_actions);
    let out = String::from_utf8(buf).unwrap();
    assert!(
        out.contains("[OK     ]  rulesets/my-ruleset (id=42)"),
        "missing ruleset ok: {out}"
    );
}

// ------------------------------------------------------------------
// print_preview: full ruleset with conditions, PR rule, status checks
// ------------------------------------------------------------------

#[test]
fn print_preview_ruleset_add_with_full_spec() {
    console::set_colors_enabled(false);
    let rs = Ruleset {
        name: String::from("full-ruleset"),
        target: Some(String::from("branch")),
        enforcement: Some(String::from("active")),
        bypass_actors: None,
        conditions: Some(RulesetConditions {
            ref_name: Some(RefNameCondition {
                include: Some(vec![String::from("refs/heads/main")]),
                exclude: Some(vec![String::from("refs/heads/temp")]),
            }),
        }),
        rules: Some(RulesetRules {
            non_fast_forward: Some(true),
            deletion: Some(false),
            creation: None,
            required_linear_history: None,
            required_signatures: Some(true),
            pull_request: Some(PullRequestRule {
                required_approving_review_count: Some(2),
                dismiss_stale_reviews_on_push: Some(true),
                require_code_owner_review: Some(false),
                require_last_push_approval: Some(false),
                required_review_thread_resolution: Some(true),
                allowed_merge_methods: Some(vec![String::from("squash")]),
            }),
            required_status_checks: Some(RequiredStatusChecks {
                strict_required_status_checks_policy: Some(true),
                contexts: Some(vec![StatusCheckContext {
                    context: String::from("ci/test"),
                    app: None,
                    integration_id: None,
                }]),
            }),
        }),
    };
    let changes = vec![SpecChange::RulesetAdd {
        name: String::from("full-ruleset"),
        spec: Box::new(rs),
    }];
    let mut buf: Vec<u8> = Vec::new();
    let (_, has_actions) = print_preview(&mut buf, &changes, "owner/repo").unwrap();
    assert!(has_actions);
    let out = String::from_utf8(buf).unwrap();
    assert!(out.contains("target: branch"), "missing target: {out}");
    assert!(
        out.contains("rules.pull_request:"),
        "missing PR rule: {out}"
    );
    assert!(
        out.contains("required_approving_review_count"),
        "missing review count: {out}"
    );
    assert!(
        out.contains("rules.required_status_checks:"),
        "missing status checks: {out}"
    );
    assert!(
        out.contains("conditions.ref_name.include"),
        "missing conditions: {out}"
    );
}

#[test]
fn print_preview_ruleset_update_with_live_data() {
    // Exercise write_ruleset_field with matches=Some(true/false)
    console::set_colors_enabled(false);
    let rs = Ruleset {
        name: String::from("protect"),
        target: Some(String::from("branch")),
        enforcement: Some(String::from("active")),
        bypass_actors: None,
        conditions: None,
        rules: None,
    };
    let live = serde_json::json!({
        "target": "branch",
        "enforcement": "disabled",
        "rules": []
    });
    let changes = vec![SpecChange::RulesetUpdate {
        id: 1,
        name: String::from("protect"),
        spec: Box::new(rs),
        live: Some(Box::new(live)),
    }];
    let mut buf: Vec<u8> = Vec::new();
    print_preview(&mut buf, &changes, "owner/repo").unwrap();
    let out = String::from_utf8(buf).unwrap();
    // target matches → OK tag; enforcement differs → CHANGED
    assert!(
        out.contains("[OK     ]  target"),
        "missing target ok: {out}"
    );
    assert!(
        out.contains("[CHANGED]  enforcement"),
        "missing enforcement changed: {out}"
    );
}

// ------------------------------------------------------------------
// apply: label update
// ------------------------------------------------------------------

#[test]
fn apply_updates_label() {
    let spec = make_spec();
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::LabelUpdate {
        name: String::from("bug"),
        old_color: String::from("ff0000"),
        old_description: None,
        new_color: String::from("0000ff"),
        new_description: Some(String::from("A bug")),
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    assert_eq!(*client.updated_labels.borrow(), vec!["bug"]);
}

// ------------------------------------------------------------------
// apply: core fields (description, topics, release_immutability)
// ------------------------------------------------------------------

#[test]
fn apply_core_fields_description_change() {
    let spec = Spec {
        description: Some(String::from("new")),
        ..make_spec()
    };
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::FieldChanged {
        field: String::from("description"),
        old: String::from("old"),
        new: String::from("new"),
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let patches = client.applied_patches.borrow();
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0]["description"], "new");
}

#[test]
fn apply_core_fields_topics_change() {
    let spec = Spec {
        topics: Some(vec![String::from("rust")]),
        ..make_spec()
    };
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::FieldChanged {
        field: String::from("topics"),
        old: String::from("[go]"),
        new: String::from("[rust]"),
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    assert_eq!(
        *client.applied_topics.borrow(),
        Some(vec![String::from("rust")])
    );
}

#[test]
fn apply_core_fields_release_immutability_change() {
    let spec = make_spec();
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::FieldChanged {
        field: String::from("release_immutability"),
        old: String::from("false"),
        new: String::from("true"),
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let calls = client.put_release_immutability_calls.borrow();
    assert_eq!(*calls, vec![true]);
}

// ------------------------------------------------------------------
// apply: actions (selected_actions path)
// ------------------------------------------------------------------

#[test]
fn apply_selected_actions_only_change() {
    // When only selected_actions changed (not actions permissions),
    // apply_selected_actions_body should be called directly.
    let spec = Spec {
        actions: Some(Actions {
            enabled: None,
            allowed_actions: None,
            sha_pinning_required: None,
            workflow_permissions: None,
            can_approve_pull_requests: None,
            selected_actions: Some(SelectedActions {
                github_owned_allowed: Some(true),
                patterns_allowed: Some(vec![String::from("actions/*")]),
            }),
            fork_pr_approval: None,
        }),
        ..make_spec()
    };
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::FieldChanged {
        field: String::from("actions.selected_actions.github_owned_allowed"),
        old: String::from("false"),
        new: String::from("true"),
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    // selected_actions put should have been called (no actions permissions put)
    assert!(client.applied_patches.borrow().is_empty());
}

// ------------------------------------------------------------------
// apply: bypass actor role-based ruleset
// ------------------------------------------------------------------

#[test]
fn apply_creates_ruleset_with_role_bypass_actor() {
    use gh_sync_manifest::BypassActor;
    let rs = Ruleset {
        name: String::from("with-bypass"),
        target: None,
        enforcement: None,
        bypass_actors: Some(vec![BypassActor {
            role: Some(String::from("admin")),
            team: None,
            app: None,
            org_admin: None,
            custom_role: None,
            bypass_mode: Some(String::from("always")),
        }]),
        conditions: None,
        rules: None,
    };
    let spec = Spec {
        rulesets: Some(vec![rs.clone()]),
        ..make_spec()
    };
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::RulesetAdd {
        name: String::from("with-bypass"),
        spec: Box::new(rs),
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let created = client.created_rulesets.borrow();
    assert_eq!(created.len(), 1);
    let actors = created[0]["bypass_actors"].as_array().unwrap();
    assert_eq!(actors[0]["actor_type"], "RepositoryRole");
    assert_eq!(actors[0]["actor_id"], 5); // admin = 5
}

#[test]
fn apply_creates_ruleset_with_org_admin_bypass_actor() {
    use gh_sync_manifest::BypassActor;
    let rs = Ruleset {
        name: String::from("org-admin-bypass"),
        target: None,
        enforcement: None,
        bypass_actors: Some(vec![BypassActor {
            role: None,
            team: None,
            app: None,
            org_admin: Some(true),
            custom_role: None,
            bypass_mode: None,
        }]),
        conditions: None,
        rules: None,
    };
    let spec = Spec {
        rulesets: Some(vec![rs.clone()]),
        ..make_spec()
    };
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::RulesetAdd {
        name: String::from("org-admin-bypass"),
        spec: Box::new(rs),
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let created = client.created_rulesets.borrow();
    let actors = created[0]["bypass_actors"].as_array().unwrap();
    assert_eq!(actors[0]["actor_type"], "OrganizationAdmin");
    assert_eq!(actors[0]["actor_id"], 1);
}

// ------------------------------------------------------------------
// apply: bypass actor — team, app, custom_role, unknown-role error
// ------------------------------------------------------------------

#[test]
fn apply_creates_ruleset_with_team_bypass_actor() {
    use gh_sync_manifest::BypassActor;
    let rs = Ruleset {
        name: String::from("team-bypass"),
        target: None,
        enforcement: None,
        bypass_actors: Some(vec![BypassActor {
            role: None,
            team: Some(String::from("ops")),
            app: None,
            org_admin: None,
            custom_role: None,
            bypass_mode: None,
        }]),
        conditions: None,
        rules: None,
    };
    let spec = Spec {
        rulesets: Some(vec![rs.clone()]),
        ..make_spec()
    };
    let mut client = MockRepoClient::new("owner/repo");
    client.team_ids.insert(String::from("ops"), 42);
    let changes = vec![SpecChange::RulesetAdd {
        name: String::from("team-bypass"),
        spec: Box::new(rs),
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let created = client.created_rulesets.borrow();
    let actors = created[0]["bypass_actors"].as_array().unwrap();
    assert_eq!(actors[0]["actor_type"], "Team");
    assert_eq!(actors[0]["actor_id"], 42);
}

#[test]
fn apply_creates_ruleset_with_app_bypass_actor() {
    use gh_sync_manifest::BypassActor;
    let rs = Ruleset {
        name: String::from("app-bypass"),
        target: None,
        enforcement: None,
        bypass_actors: Some(vec![BypassActor {
            role: None,
            team: None,
            app: Some(String::from("my-bot")),
            org_admin: None,
            custom_role: None,
            bypass_mode: Some(String::from("pull_request")),
        }]),
        conditions: None,
        rules: None,
    };
    let spec = Spec {
        rulesets: Some(vec![rs.clone()]),
        ..make_spec()
    };
    let mut client = MockRepoClient::new("owner/repo");
    client.app_ids.insert(String::from("my-bot"), 99);
    let changes = vec![SpecChange::RulesetAdd {
        name: String::from("app-bypass"),
        spec: Box::new(rs),
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let created = client.created_rulesets.borrow();
    let actors = created[0]["bypass_actors"].as_array().unwrap();
    assert_eq!(actors[0]["actor_type"], "Integration");
    assert_eq!(actors[0]["actor_id"], 99);
    assert_eq!(actors[0]["bypass_mode"], "pull_request");
}

#[test]
fn apply_creates_ruleset_with_custom_role_bypass_actor() {
    use gh_sync_manifest::BypassActor;
    let rs = Ruleset {
        name: String::from("custom-role-bypass"),
        target: None,
        enforcement: None,
        bypass_actors: Some(vec![BypassActor {
            role: None,
            team: None,
            app: None,
            org_admin: None,
            custom_role: Some(String::from("deployer")),
            bypass_mode: None,
        }]),
        conditions: None,
        rules: None,
    };
    let spec = Spec {
        rulesets: Some(vec![rs.clone()]),
        ..make_spec()
    };
    let mut client = MockRepoClient::new("owner/repo");
    client.custom_role_ids.insert(String::from("deployer"), 200);
    let changes = vec![SpecChange::RulesetAdd {
        name: String::from("custom-role-bypass"),
        spec: Box::new(rs),
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let created = client.created_rulesets.borrow();
    let actors = created[0]["bypass_actors"].as_array().unwrap();
    assert_eq!(actors[0]["actor_type"], "RepositoryRole");
    assert_eq!(actors[0]["actor_id"], 200);
}

#[test]
fn apply_bypass_actor_unknown_role_returns_error() {
    use gh_sync_manifest::BypassActor;
    let rs = Ruleset {
        name: String::from("bad-role"),
        target: None,
        enforcement: None,
        bypass_actors: Some(vec![BypassActor {
            role: Some(String::from("superadmin")),
            team: None,
            app: None,
            org_admin: None,
            custom_role: None,
            bypass_mode: None,
        }]),
        conditions: None,
        rules: None,
    };
    let spec = Spec {
        rulesets: Some(vec![rs.clone()]),
        ..make_spec()
    };
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::RulesetAdd {
        name: String::from("bad-role"),
        spec: Box::new(rs),
    }];
    assert!(apply_changes(&changes, &spec, "owner/repo", &client).is_err());
}

#[test]
fn apply_bypass_actor_no_type_returns_error() {
    use gh_sync_manifest::BypassActor;
    let rs = Ruleset {
        name: String::from("no-type"),
        target: None,
        enforcement: None,
        bypass_actors: Some(vec![BypassActor {
            role: None,
            team: None,
            app: None,
            org_admin: None,
            custom_role: None,
            bypass_mode: None,
        }]),
        conditions: None,
        rules: None,
    };
    let spec = Spec {
        rulesets: Some(vec![rs.clone()]),
        ..make_spec()
    };
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::RulesetAdd {
        name: String::from("no-type"),
        spec: Box::new(rs),
    }];
    assert!(apply_changes(&changes, &spec, "owner/repo", &client).is_err());
}

// ------------------------------------------------------------------
// apply: ruleset with conditions and status checks rules
// ------------------------------------------------------------------

#[test]
fn apply_creates_ruleset_with_conditions_and_status_checks() {
    let rs = Ruleset {
        name: String::from("full-ruleset"),
        target: Some(String::from("branch")),
        enforcement: Some(String::from("active")),
        bypass_actors: None,
        conditions: Some(RulesetConditions {
            ref_name: Some(RefNameCondition {
                include: Some(vec![String::from("refs/heads/main")]),
                exclude: Some(vec![String::from("refs/heads/dev")]),
            }),
        }),
        rules: Some(RulesetRules {
            non_fast_forward: None,
            deletion: None,
            creation: None,
            required_linear_history: None,
            required_signatures: None,
            pull_request: None,
            required_status_checks: Some(RequiredStatusChecks {
                strict_required_status_checks_policy: Some(true),
                contexts: Some(vec![
                    StatusCheckContext {
                        context: String::from("ci/build"),
                        integration_id: Some(12345),
                        app: None,
                    },
                    StatusCheckContext {
                        context: String::from("ci/test"),
                        integration_id: None,
                        app: Some(String::from("github-actions")),
                    },
                ]),
            }),
        }),
    };
    let spec = Spec {
        rulesets: Some(vec![rs.clone()]),
        ..make_spec()
    };
    let mut client = MockRepoClient::new("owner/repo");
    client.app_ids.insert(String::from("github-actions"), 15368);
    let changes = vec![SpecChange::RulesetAdd {
        name: String::from("full-ruleset"),
        spec: Box::new(rs),
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let created = client.created_rulesets.borrow();
    assert_eq!(created.len(), 1);
    let body = &created[0];
    assert_eq!(
        body["conditions"]["ref_name"]["include"][0],
        "refs/heads/main"
    );
    assert_eq!(
        body["conditions"]["ref_name"]["exclude"][0],
        "refs/heads/dev"
    );
    let rules = body["rules"].as_array().unwrap();
    let sc_rule = rules
        .iter()
        .find(|r| r["type"] == "required_status_checks")
        .unwrap();
    assert_eq!(
        sc_rule["parameters"]["strict_required_status_checks_policy"],
        true
    );
    let ctxs = sc_rule["parameters"]["required_status_checks"]
        .as_array()
        .unwrap();
    let build_ctx = ctxs.iter().find(|c| c["context"] == "ci/build").unwrap();
    assert_eq!(build_ctx["integration_id"], 12345);
    let test_ctx = ctxs.iter().find(|c| c["context"] == "ci/test").unwrap();
    assert_eq!(test_ctx["integration_id"], 15368);
}

#[test]
fn apply_creates_ruleset_with_pr_rule_and_merge_methods() {
    let rs = Ruleset {
        name: String::from("pr-rule"),
        target: None,
        enforcement: None,
        bypass_actors: None,
        conditions: None,
        rules: Some(RulesetRules {
            non_fast_forward: None,
            deletion: None,
            creation: None,
            required_linear_history: None,
            required_signatures: None,
            pull_request: Some(PullRequestRule {
                required_approving_review_count: Some(2),
                dismiss_stale_reviews_on_push: Some(true),
                require_code_owner_review: Some(true),
                require_last_push_approval: Some(false),
                required_review_thread_resolution: Some(true),
                allowed_merge_methods: Some(vec![String::from("squash"), String::from("merge")]),
            }),
            required_status_checks: None,
        }),
    };
    let spec = Spec {
        rulesets: Some(vec![rs.clone()]),
        ..make_spec()
    };
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::RulesetAdd {
        name: String::from("pr-rule"),
        spec: Box::new(rs),
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let created = client.created_rulesets.borrow();
    let rules = created[0]["rules"].as_array().unwrap();
    let pr_rule = rules.iter().find(|r| r["type"] == "pull_request").unwrap();
    assert_eq!(pr_rule["parameters"]["required_approving_review_count"], 2);
    assert_eq!(pr_rule["parameters"]["dismiss_stale_reviews_on_push"], true);
    assert_eq!(pr_rule["parameters"]["require_code_owner_review"], true);
    assert_eq!(pr_rule["parameters"]["require_last_push_approval"], false);
    assert_eq!(
        pr_rule["parameters"]["required_review_thread_resolution"],
        true
    );
    let methods = pr_rule["parameters"]["allowed_merge_methods"]
        .as_array()
        .unwrap();
    assert_eq!(methods.len(), 2);
}

// ------------------------------------------------------------------
// apply: branch protection Update and Remove
// ------------------------------------------------------------------

#[test]
fn apply_updates_branch_protection() {
    let bp = BranchProtection {
        pattern: String::from("main"),
        required_reviews: Some(2),
        dismiss_stale_reviews: None,
        require_code_owner_reviews: None,
        require_status_checks: None,
        enforce_admins: Some(true),
        allow_force_pushes: Some(false),
        allow_deletions: Some(false),
    };
    let spec = Spec {
        branch_protection: Some(vec![bp.clone()]),
        ..make_spec()
    };
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::BranchProtectionUpdate { spec: Box::new(bp) }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let put = client.put_branch_protections.borrow();
    assert_eq!(put.len(), 1);
    assert_eq!(put[0].0, "main");
    assert_eq!(
        put[0].1["required_pull_request_reviews"]["required_approving_review_count"],
        2
    );
}

#[test]
fn apply_removes_branch_protection() {
    let spec = make_spec();
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::BranchProtectionRemove {
        pattern: String::from("feature"),
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let deleted = client.deleted_branch_protections.borrow();
    assert_eq!(*deleted, vec!["feature"]);
}

// ------------------------------------------------------------------
// apply: actions — fork_pr_approval path
// ------------------------------------------------------------------

#[test]
fn apply_fork_pr_approval_change() {
    let spec = Spec {
        actions: Some(Actions {
            enabled: None,
            allowed_actions: None,
            sha_pinning_required: None,
            workflow_permissions: None,
            can_approve_pull_requests: None,
            selected_actions: None,
            fork_pr_approval: Some(String::from("always")),
        }),
        ..make_spec()
    };
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::FieldChanged {
        field: String::from("actions.fork_pr_approval"),
        old: String::from("never"),
        new: String::from("always"),
    }];
    // Should not fail
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
}

// ------------------------------------------------------------------
// apply: core_field_to_patch — remaining match arms
// ------------------------------------------------------------------

#[test]
fn core_field_to_patch_features_wiki() {
    let (key, val) = core_field_to_patch("features.wiki", "true").unwrap();
    assert_eq!(key, "has_wiki");
    assert_eq!(val, serde_json::json!(true));
}

#[test]
fn core_field_to_patch_features_discussions() {
    let (key, val) = core_field_to_patch("features.discussions", "false").unwrap();
    assert_eq!(key, "has_discussions");
    assert_eq!(val, serde_json::json!(false));
}

#[test]
fn core_field_to_patch_web_commit_signoff() {
    let (key, val) = core_field_to_patch("web_commit_signoff_required", "true").unwrap();
    assert_eq!(key, "web_commit_signoff_required");
    assert_eq!(val, serde_json::json!(true));
}

#[test]
fn core_field_to_patch_squash_merge_commit_title() {
    let (key, val) =
        core_field_to_patch("merge_strategy.squash_merge_commit_title", "PR_TITLE").unwrap();
    assert_eq!(key, "squash_merge_commit_title");
    assert_eq!(val, serde_json::json!("PR_TITLE"));
}

#[test]
fn core_field_to_patch_squash_merge_commit_message() {
    let (key, val) =
        core_field_to_patch("merge_strategy.squash_merge_commit_message", "BLANK").unwrap();
    assert_eq!(key, "squash_merge_commit_message");
    assert_eq!(val, serde_json::json!("BLANK"));
}

#[test]
fn core_field_to_patch_merge_commit_title() {
    let (key, val) =
        core_field_to_patch("merge_strategy.merge_commit_title", "MERGE_MESSAGE").unwrap();
    assert_eq!(key, "merge_commit_title");
    assert_eq!(val, serde_json::json!("MERGE_MESSAGE"));
}

#[test]
fn core_field_to_patch_merge_commit_message() {
    let (key, val) = core_field_to_patch("merge_strategy.merge_commit_message", "PR_BODY").unwrap();
    assert_eq!(key, "merge_commit_message");
    assert_eq!(val, serde_json::json!("PR_BODY"));
}

#[test]
fn core_field_to_patch_visibility() {
    let (key, val) = core_field_to_patch("visibility", "private").unwrap();
    assert_eq!(key, "visibility");
    assert_eq!(val, serde_json::json!("private"));
}

#[test]
fn core_field_to_patch_archived() {
    let (key, val) = core_field_to_patch("archived", "true").unwrap();
    assert_eq!(key, "archived");
    assert_eq!(val, serde_json::json!(true));
}

// ------------------------------------------------------------------
// compare: visibility, archived, web_commit_signoff_required
// ------------------------------------------------------------------

#[test]
fn compare_visibility_changed() {
    let mut client = MockRepoClient::new("owner/repo");
    client.repo_data.visibility = String::from("public");
    let spec = Spec {
        visibility: Some(String::from("private")),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldChanged { field, new, .. }
        if field == "visibility" && new == "private"
    )));
}

#[test]
fn compare_visibility_unchanged() {
    let mut client = MockRepoClient::new("owner/repo");
    client.repo_data.visibility = String::from("private");
    let spec = Spec {
        visibility: Some(String::from("private")),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(
        changes
            .iter()
            .any(|c| matches!(c, SpecChange::FieldOk { field, .. } if field == "visibility"))
    );
}

#[test]
fn compare_archived_changed() {
    let mut client = MockRepoClient::new("owner/repo");
    client.repo_data.archived = false;
    let spec = Spec {
        archived: Some(true),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldChanged { field, new, .. }
        if field == "archived" && new == "true"
    )));
}

#[test]
fn compare_web_commit_signoff_changed() {
    let mut client = MockRepoClient::new("owner/repo");
    client.repo_data.web_commit_signoff_required = false;
    let spec = Spec {
        web_commit_signoff_required: Some(true),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldChanged { field, .. }
        if field == "web_commit_signoff_required"
    )));
}

#[test]
fn compare_release_immutability_endpoint_unavailable() {
    // When fetch_release_immutability returns None, we get FieldOk with a skip message.
    let client = MockRepoClient::new("owner/repo"); // release_immutability = None
    let spec = Spec {
        release_immutability: Some(true),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldOk { field, value }
        if field == "release_immutability" && value.contains("skipped")
    )));
}

// ------------------------------------------------------------------
// compare: actions — patterns_allowed, workflow, fork_pr_approval, sha_pinning
// ------------------------------------------------------------------

#[test]
fn compare_actions_patterns_allowed_unchanged() {
    let mut client = MockRepoClient::new("owner/repo");
    client.actions_permissions.allowed_actions = Some(String::from("selected"));
    client.selected_actions.patterns_allowed =
        Some(vec![String::from("actions/*"), String::from("github/*")]);
    let spec = Spec {
        actions: Some(Actions {
            enabled: None,
            allowed_actions: None,
            sha_pinning_required: None,
            workflow_permissions: None,
            can_approve_pull_requests: None,
            selected_actions: Some(SelectedActions {
                github_owned_allowed: None,
                patterns_allowed: Some(vec![String::from("github/*"), String::from("actions/*")]),
            }),
            fork_pr_approval: None,
        }),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldOk { field, .. }
        if field == "actions.selected_actions.patterns_allowed"
    )));
}

#[test]
fn compare_actions_patterns_allowed_changed() {
    let mut client = MockRepoClient::new("owner/repo");
    client.actions_permissions.allowed_actions = Some(String::from("selected"));
    client.selected_actions.patterns_allowed = Some(vec![String::from("actions/*")]);
    let spec = Spec {
        actions: Some(Actions {
            enabled: None,
            allowed_actions: None,
            sha_pinning_required: None,
            workflow_permissions: None,
            can_approve_pull_requests: None,
            selected_actions: Some(SelectedActions {
                github_owned_allowed: None,
                patterns_allowed: Some(vec![String::from("actions/*"), String::from("github/*")]),
            }),
            fork_pr_approval: None,
        }),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldChanged { field, .. }
        if field == "actions.selected_actions.patterns_allowed"
    )));
}

#[test]
fn compare_actions_workflow_permissions_and_can_approve_changed() {
    let mut client = MockRepoClient::new("owner/repo");
    client.workflow_permissions.default_workflow_permissions = Some(String::from("read"));
    client.workflow_permissions.can_approve_pull_request_reviews = false;
    let spec = Spec {
        actions: Some(Actions {
            enabled: None,
            allowed_actions: None,
            sha_pinning_required: None,
            workflow_permissions: Some(String::from("write")),
            can_approve_pull_requests: Some(true),
            selected_actions: None,
            fork_pr_approval: None,
        }),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldChanged { field, .. }
        if field == "actions.workflow_permissions"
    )));
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldChanged { field, .. }
        if field == "actions.can_approve_pull_requests"
    )));
}

#[test]
fn compare_actions_fork_pr_approval_changed() {
    let mut client = MockRepoClient::new("owner/repo");
    client.fork_pr_approval = Some(String::from("never"));
    let spec = Spec {
        actions: Some(Actions {
            enabled: None,
            allowed_actions: None,
            sha_pinning_required: None,
            workflow_permissions: None,
            can_approve_pull_requests: None,
            selected_actions: None,
            fork_pr_approval: Some(String::from("always")),
        }),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldChanged { field, new, .. }
        if field == "actions.fork_pr_approval" && new == "always"
    )));
}

#[test]
fn compare_actions_fork_pr_approval_endpoint_unavailable() {
    // fork_pr_approval = None means endpoint is unavailable → FieldOk with skip message
    let client = MockRepoClient::new("owner/repo"); // fork_pr_approval = None
    let spec = Spec {
        actions: Some(Actions {
            enabled: None,
            allowed_actions: None,
            sha_pinning_required: None,
            workflow_permissions: None,
            can_approve_pull_requests: None,
            selected_actions: None,
            fork_pr_approval: Some(String::from("always")),
        }),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldOk { field, value }
        if field == "actions.fork_pr_approval" && value.contains("skipped")
    )));
}

#[test]
fn compare_actions_sha_pinning_required_changed() {
    let mut client = MockRepoClient::new("owner/repo");
    client.actions_permissions.sha_pinning_required = Some(false);
    let spec = Spec {
        actions: Some(Actions {
            enabled: None,
            allowed_actions: None,
            sha_pinning_required: Some(true),
            workflow_permissions: None,
            can_approve_pull_requests: None,
            selected_actions: None,
            fork_pr_approval: None,
        }),
        ..make_spec()
    };
    let changes = compare(&spec, "owner/repo", &client).unwrap();
    assert!(changes.iter().any(|c| matches!(
        c,
        SpecChange::FieldChanged { field, new, .. }
        if field == "actions.sha_pinning_required" && new == "true"
    )));
}

// ------------------------------------------------------------------
// compare: ruleset_rules_match — status checks
// ------------------------------------------------------------------

#[test]
fn ruleset_rules_match_status_checks_match() {
    let rules = RulesetRules {
        non_fast_forward: None,
        deletion: None,
        creation: None,
        required_linear_history: None,
        required_signatures: None,
        pull_request: None,
        required_status_checks: Some(RequiredStatusChecks {
            strict_required_status_checks_policy: Some(true),
            contexts: Some(vec![StatusCheckContext {
                context: String::from("ci"),
                integration_id: None,
                app: None,
            }]),
        }),
    };
    let live_rules = vec![serde_json::json!({
        "type": "required_status_checks",
        "parameters": {
            "strict_required_status_checks_policy": true,
            "required_status_checks": [{"context": "ci", "integration_id": 15368}]
        }
    })];
    assert!(ruleset_rules_match(&rules, &live_rules));
}

#[test]
fn ruleset_rules_match_status_checks_mismatch() {
    let rules = RulesetRules {
        non_fast_forward: None,
        deletion: None,
        creation: None,
        required_linear_history: None,
        required_signatures: None,
        pull_request: None,
        required_status_checks: Some(RequiredStatusChecks {
            strict_required_status_checks_policy: Some(false),
            contexts: Some(vec![StatusCheckContext {
                context: String::from("ci"),
                integration_id: None,
                app: None,
            }]),
        }),
    };
    let live_rules = vec![serde_json::json!({
        "type": "required_status_checks",
        "parameters": {
            "strict_required_status_checks_policy": true,
            "required_status_checks": [{"context": "ci", "integration_id": 15368}]
        }
    })];
    assert!(!ruleset_rules_match(&rules, &live_rules));
}

// ------------------------------------------------------------------
// compare: protection_matches — remaining fields
// ------------------------------------------------------------------

#[test]
fn protection_matches_dismiss_stale_mismatch() {
    let spec = BranchProtection {
        pattern: String::from("main"),
        required_reviews: None,
        dismiss_stale_reviews: Some(true),
        require_code_owner_reviews: None,
        require_status_checks: None,
        enforce_admins: None,
        allow_force_pushes: None,
        allow_deletions: None,
    };
    let live = super::BranchProtectionApi {
        required_reviews: None,
        dismiss_stale_reviews: false,
        require_code_owner_reviews: false,
        strict_status_checks: false,
        status_check_contexts: vec![],
        enforce_admins: false,
        allow_force_pushes: false,
        allow_deletions: false,
    };
    assert!(!protection_matches(&spec, &live));
}

#[test]
fn protection_matches_require_code_owner_mismatch() {
    let spec = BranchProtection {
        pattern: String::from("main"),
        required_reviews: None,
        dismiss_stale_reviews: None,
        require_code_owner_reviews: Some(true),
        require_status_checks: None,
        enforce_admins: None,
        allow_force_pushes: None,
        allow_deletions: None,
    };
    let live = super::BranchProtectionApi {
        required_reviews: None,
        dismiss_stale_reviews: false,
        require_code_owner_reviews: false,
        strict_status_checks: false,
        status_check_contexts: vec![],
        enforce_admins: false,
        allow_force_pushes: false,
        allow_deletions: false,
    };
    assert!(!protection_matches(&spec, &live));
}

#[test]
fn protection_matches_require_status_checks_strict_mismatch() {
    let spec = BranchProtection {
        pattern: String::from("main"),
        required_reviews: None,
        dismiss_stale_reviews: None,
        require_code_owner_reviews: None,
        require_status_checks: Some(BranchProtectionStatusChecks {
            strict: Some(true),
            contexts: None,
        }),
        enforce_admins: None,
        allow_force_pushes: None,
        allow_deletions: None,
    };
    let live = super::BranchProtectionApi {
        required_reviews: None,
        dismiss_stale_reviews: false,
        require_code_owner_reviews: false,
        strict_status_checks: false,
        status_check_contexts: vec![],
        enforce_admins: false,
        allow_force_pushes: false,
        allow_deletions: false,
    };
    assert!(!protection_matches(&spec, &live));
}

#[test]
fn protection_matches_status_check_contexts_mismatch() {
    let spec = BranchProtection {
        pattern: String::from("main"),
        required_reviews: None,
        dismiss_stale_reviews: None,
        require_code_owner_reviews: None,
        require_status_checks: Some(BranchProtectionStatusChecks {
            strict: None,
            contexts: Some(vec![String::from("ci/build"), String::from("ci/test")]),
        }),
        enforce_admins: None,
        allow_force_pushes: None,
        allow_deletions: None,
    };
    let live = super::BranchProtectionApi {
        required_reviews: None,
        dismiss_stale_reviews: false,
        require_code_owner_reviews: false,
        strict_status_checks: false,
        status_check_contexts: vec![String::from("ci/build")],
        enforce_admins: false,
        allow_force_pushes: false,
        allow_deletions: false,
    };
    assert!(!protection_matches(&spec, &live));
}

#[test]
fn protection_matches_enforce_admins_false_mismatch() {
    let spec = BranchProtection {
        pattern: String::from("main"),
        required_reviews: None,
        dismiss_stale_reviews: None,
        require_code_owner_reviews: None,
        require_status_checks: None,
        enforce_admins: Some(true),
        allow_force_pushes: None,
        allow_deletions: None,
    };
    let live = super::BranchProtectionApi {
        required_reviews: None,
        dismiss_stale_reviews: false,
        require_code_owner_reviews: false,
        strict_status_checks: false,
        status_check_contexts: vec![],
        enforce_admins: false,
        allow_force_pushes: false,
        allow_deletions: false,
    };
    assert!(!protection_matches(&spec, &live));
}

#[test]
fn protection_matches_allow_force_pushes_mismatch() {
    let spec = BranchProtection {
        pattern: String::from("main"),
        required_reviews: None,
        dismiss_stale_reviews: None,
        require_code_owner_reviews: None,
        require_status_checks: None,
        enforce_admins: None,
        allow_force_pushes: Some(true),
        allow_deletions: None,
    };
    let live = super::BranchProtectionApi {
        required_reviews: None,
        dismiss_stale_reviews: false,
        require_code_owner_reviews: false,
        strict_status_checks: false,
        status_check_contexts: vec![],
        enforce_admins: false,
        allow_force_pushes: false,
        allow_deletions: false,
    };
    assert!(!protection_matches(&spec, &live));
}

#[test]
fn protection_matches_allow_deletions_mismatch() {
    let spec = BranchProtection {
        pattern: String::from("main"),
        required_reviews: None,
        dismiss_stale_reviews: None,
        require_code_owner_reviews: None,
        require_status_checks: None,
        enforce_admins: None,
        allow_force_pushes: None,
        allow_deletions: Some(true),
    };
    let live = super::BranchProtectionApi {
        required_reviews: None,
        dismiss_stale_reviews: false,
        require_code_owner_reviews: false,
        strict_status_checks: false,
        status_check_contexts: vec![],
        enforce_admins: false,
        allow_force_pushes: false,
        allow_deletions: false,
    };
    assert!(!protection_matches(&spec, &live));
}

// ------------------------------------------------------------------
// print_preview: RulesetUpdate with full live JSON
// ------------------------------------------------------------------

#[test]
fn print_preview_ruleset_update_with_full_live_data() {
    let rs = Ruleset {
        name: String::from("protect-main"),
        target: Some(String::from("branch")),
        enforcement: Some(String::from("active")),
        bypass_actors: None,
        conditions: Some(RulesetConditions {
            ref_name: Some(RefNameCondition {
                include: Some(vec![String::from("refs/heads/main")]),
                exclude: Some(vec![]),
            }),
        }),
        rules: Some(RulesetRules {
            non_fast_forward: Some(true),
            deletion: Some(false),
            creation: None,
            required_linear_history: None,
            required_signatures: None,
            pull_request: Some(PullRequestRule {
                required_approving_review_count: Some(1),
                dismiss_stale_reviews_on_push: Some(false),
                require_code_owner_review: Some(true),
                require_last_push_approval: None,
                required_review_thread_resolution: None,
                allowed_merge_methods: Some(vec![String::from("squash")]),
            }),
            required_status_checks: Some(RequiredStatusChecks {
                strict_required_status_checks_policy: Some(true),
                contexts: Some(vec![StatusCheckContext {
                    context: String::from("ci"),
                    integration_id: None,
                    app: None,
                }]),
            }),
        }),
    };
    // Live JSON that matches some fields and mismatches others.
    let live_json = serde_json::json!({
        "target": "branch",
        "enforcement": "disabled",  // mismatch
        "conditions": {
            "ref_name": {
                "include": ["refs/heads/main"],
                "exclude": []
            }
        },
        "rules": [
            { "type": "non_fast_forward" },
            {
                "type": "pull_request",
                "parameters": {
                    "required_approving_review_count": 2,  // mismatch
                    "dismiss_stale_reviews_on_push": false,
                    "require_code_owner_review": true,
                    "allowed_merge_methods": ["squash"]
                }
            },
            {
                "type": "required_status_checks",
                "parameters": {
                    "strict_required_status_checks_policy": true,
                    "required_status_checks": [{"context": "ci", "integration_id": 15368}]
                }
            }
        ]
    });
    let changes = vec![SpecChange::RulesetUpdate {
        id: 101,
        name: String::from("protect-main"),
        spec: Box::new(rs),
        live: Some(Box::new(live_json)),
    }];
    let mut buf: Vec<u8> = Vec::new();
    print_preview(&mut buf, &changes, "owner/repo").unwrap();
    let output = String::from_utf8(buf).unwrap();
    // Verify key content appears in the output
    assert!(output.contains("protect-main"));
    assert!(output.contains("enforcement"));
    assert!(output.contains("required_approving_review_count"));
    assert!(output.contains("require_code_owner_review"));
    assert!(output.contains("allowed_merge_methods"));
    assert!(output.contains("strict_required_status_checks_policy"));
    assert!(output.contains("contexts"));
    assert!(output.contains("conditions.ref_name"));
}

#[test]
fn print_preview_ruleset_update_no_live_data() {
    // When live = None, tags are not shown (no match/mismatch display)
    let rs = Ruleset {
        name: String::from("no-live"),
        target: Some(String::from("tag")),
        enforcement: Some(String::from("evaluate")),
        bypass_actors: None,
        conditions: Some(RulesetConditions {
            ref_name: Some(RefNameCondition {
                include: Some(vec![String::from("refs/tags/*")]),
                exclude: None,
            }),
        }),
        rules: Some(RulesetRules {
            non_fast_forward: None,
            deletion: Some(true),
            creation: None,
            required_linear_history: None,
            required_signatures: Some(true),
            pull_request: None,
            required_status_checks: None,
        }),
    };
    let changes = vec![SpecChange::RulesetUpdate {
        id: 202,
        name: String::from("no-live"),
        spec: Box::new(rs),
        live: None,
    }];
    let mut buf: Vec<u8> = Vec::new();
    print_preview(&mut buf, &changes, "owner/repo").unwrap();
    let output = String::from_utf8(buf).unwrap();
    assert!(output.contains("no-live"));
    assert!(output.contains("target"));
    assert!(output.contains("enforcement"));
    assert!(output.contains("conditions.ref_name"));
    assert!(output.contains("rules.deletion"));
    assert!(output.contains("rules.required_signatures"));
}

// ------------------------------------------------------------------
// apply: merge-method/title stripping (GitHub API 422 guard)
// ------------------------------------------------------------------

#[test]
fn apply_core_fields_strips_merge_commit_fields_when_disabled() {
    let spec = make_spec();
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![
        SpecChange::FieldChanged {
            field: String::from("merge_strategy.allow_merge_commit"),
            old: String::from("true"),
            new: String::from("false"),
        },
        SpecChange::FieldChanged {
            field: String::from("merge_strategy.merge_commit_title"),
            old: String::from("MERGE_MESSAGE"),
            new: String::from("PR_TITLE"),
        },
        SpecChange::FieldChanged {
            field: String::from("merge_strategy.merge_commit_message"),
            old: String::from("BLANK"),
            new: String::from("PR_TITLE"),
        },
    ];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let patches = client.applied_patches.borrow();
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0]["allow_merge_commit"], false);
    assert!(patches[0].get("merge_commit_title").is_none());
    assert!(patches[0].get("merge_commit_message").is_none());
}

#[test]
fn apply_core_fields_strips_squash_fields_when_disabled() {
    let spec = make_spec();
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![
        SpecChange::FieldChanged {
            field: String::from("merge_strategy.allow_squash_merge"),
            old: String::from("true"),
            new: String::from("false"),
        },
        SpecChange::FieldChanged {
            field: String::from("merge_strategy.squash_merge_commit_title"),
            old: String::from("COMMIT_OR_PR_TITLE"),
            new: String::from("PR_TITLE"),
        },
        SpecChange::FieldChanged {
            field: String::from("merge_strategy.squash_merge_commit_message"),
            old: String::from("BLANK"),
            new: String::from("PR_BODY"),
        },
    ];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let patches = client.applied_patches.borrow();
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0]["allow_squash_merge"], false);
    assert!(patches[0].get("squash_merge_commit_title").is_none());
    assert!(patches[0].get("squash_merge_commit_message").is_none());
}

#[test]
fn apply_core_fields_keeps_merge_commit_fields_when_enabled() {
    let spec = make_spec();
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![
        SpecChange::FieldChanged {
            field: String::from("merge_strategy.allow_merge_commit"),
            old: String::from("false"),
            new: String::from("true"),
        },
        SpecChange::FieldChanged {
            field: String::from("merge_strategy.merge_commit_title"),
            old: String::from("MERGE_MESSAGE"),
            new: String::from("PR_TITLE"),
        },
        SpecChange::FieldChanged {
            field: String::from("merge_strategy.merge_commit_message"),
            old: String::from("BLANK"),
            new: String::from("PR_TITLE"),
        },
    ];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let patches = client.applied_patches.borrow();
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0]["allow_merge_commit"], true);
    assert_eq!(patches[0]["merge_commit_title"], "PR_TITLE");
    assert_eq!(patches[0]["merge_commit_message"], "PR_TITLE");
}

#[test]
fn apply_core_fields_strips_merge_commit_fields_when_spec_already_disabled() {
    // spec says allow_merge_commit=false (live side already disabled),
    // only title is changing — must NOT reach the PATCH body.
    let spec = Spec {
        merge_strategy: Some(MergeStrategy {
            allow_merge_commit: Some(false),
            allow_squash_merge: None,
            allow_rebase_merge: None,
            allow_auto_merge: None,
            allow_update_branch: None,
            auto_delete_head_branches: None,
            merge_commit_title: Some(String::from("PR_TITLE")),
            merge_commit_message: Some(String::from("BLANK")),
            squash_merge_commit_title: None,
            squash_merge_commit_message: None,
        }),
        ..make_spec()
    };
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![
        SpecChange::FieldChanged {
            field: String::from("merge_strategy.merge_commit_title"),
            old: String::from("MERGE_MESSAGE"),
            new: String::from("PR_TITLE"),
        },
        SpecChange::FieldChanged {
            field: String::from("merge_strategy.merge_commit_message"),
            old: String::from("PR_TITLE"),
            new: String::from("BLANK"),
        },
    ];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let patches = client.applied_patches.borrow();
    // No PATCH should be sent because all changed fields were stripped.
    assert_eq!(patches.len(), 0);
}

#[test]
fn apply_core_fields_strips_squash_fields_when_spec_already_disabled() {
    let spec = Spec {
        merge_strategy: Some(MergeStrategy {
            allow_merge_commit: None,
            allow_squash_merge: Some(false),
            allow_rebase_merge: None,
            allow_auto_merge: None,
            allow_update_branch: None,
            auto_delete_head_branches: None,
            merge_commit_title: None,
            merge_commit_message: None,
            squash_merge_commit_title: Some(String::from("PR_TITLE")),
            squash_merge_commit_message: Some(String::from("BLANK")),
        }),
        ..make_spec()
    };
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![
        SpecChange::FieldChanged {
            field: String::from("merge_strategy.squash_merge_commit_title"),
            old: String::from("COMMIT_OR_PR_TITLE"),
            new: String::from("PR_TITLE"),
        },
        SpecChange::FieldChanged {
            field: String::from("merge_strategy.squash_merge_commit_message"),
            old: String::from("COMMIT_MESSAGES"),
            new: String::from("BLANK"),
        },
    ];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let patches = client.applied_patches.borrow();
    assert_eq!(patches.len(), 0);
}

#[test]
fn apply_core_fields_keeps_merge_commit_fields_when_spec_explicitly_enabled() {
    let spec = Spec {
        merge_strategy: Some(MergeStrategy {
            allow_merge_commit: Some(true),
            allow_squash_merge: None,
            allow_rebase_merge: None,
            allow_auto_merge: None,
            allow_update_branch: None,
            auto_delete_head_branches: None,
            merge_commit_title: Some(String::from("PR_TITLE")),
            merge_commit_message: None,
            squash_merge_commit_title: None,
            squash_merge_commit_message: None,
        }),
        ..make_spec()
    };
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::FieldChanged {
        field: String::from("merge_strategy.merge_commit_title"),
        old: String::from("MERGE_MESSAGE"),
        new: String::from("PR_TITLE"),
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let patches = client.applied_patches.borrow();
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0]["merge_commit_title"], "PR_TITLE");
}

#[test]
fn apply_core_fields_strips_merge_commit_fields_when_spec_is_none() {
    // When spec has no merge_strategy at all, treat as "not enabled" → strip.
    let spec = make_spec(); // merge_strategy: None
    let client = MockRepoClient::new("owner/repo");
    let changes = vec![SpecChange::FieldChanged {
        field: String::from("merge_strategy.merge_commit_title"),
        old: String::from("MERGE_MESSAGE"),
        new: String::from("PR_TITLE"),
    }];
    apply_changes(&changes, &spec, "owner/repo", &client).unwrap();
    let patches = client.applied_patches.borrow();
    assert_eq!(patches.len(), 0);
}
