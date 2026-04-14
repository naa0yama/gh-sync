//! Parse raw GitHub API JSON into typed structs.
// TODO: add per-item doc comments to satisfy `missing_docs` and `missing_errors_doc`
#![allow(missing_docs)]
#![allow(clippy::must_use_candidate)]

use super::{BranchProtectionApi, RepoApiData};

pub fn parse_repo_api_data(v: &serde_json::Value) -> RepoApiData {
    RepoApiData {
        description: v["description"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        homepage: v["homepage"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        visibility: v["visibility"].as_str().unwrap_or("public").to_lowercase(),
        archived: v["archived"].as_bool().unwrap_or(false),
        has_issues: v["has_issues"].as_bool().unwrap_or(true),
        has_projects: v["has_projects"].as_bool().unwrap_or(true),
        has_wiki: v["has_wiki"].as_bool().unwrap_or(true),
        has_discussions: v["has_discussions"].as_bool().unwrap_or(false),
        delete_branch_on_merge: v["delete_branch_on_merge"].as_bool().unwrap_or(false),
        allow_merge_commit: v["allow_merge_commit"].as_bool().unwrap_or(true),
        allow_squash_merge: v["allow_squash_merge"].as_bool().unwrap_or(true),
        allow_rebase_merge: v["allow_rebase_merge"].as_bool().unwrap_or(true),
        allow_auto_merge: v["allow_auto_merge"].as_bool().unwrap_or(false),
        allow_update_branch: v["allow_update_branch"].as_bool().unwrap_or(false),
        web_commit_signoff_required: v["web_commit_signoff_required"].as_bool().unwrap_or(false),
        squash_merge_commit_title: v["squash_merge_commit_title"].as_str().map(str::to_owned),
        squash_merge_commit_message: v["squash_merge_commit_message"].as_str().map(str::to_owned),
        merge_commit_title: v["merge_commit_title"].as_str().map(str::to_owned),
        merge_commit_message: v["merge_commit_message"].as_str().map(str::to_owned),
    }
}

pub fn parse_branch_protection_api(v: &serde_json::Value) -> BranchProtectionApi {
    let required_reviews = v
        .get("required_pull_request_reviews")
        .and_then(|r| r.get("required_approving_review_count"))
        .and_then(serde_json::Value::as_u64)
        .map(|n| u32::try_from(n).unwrap_or(u32::MAX));
    let dismiss_stale = v
        .get("required_pull_request_reviews")
        .and_then(|r| r.get("dismiss_stale_reviews"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let code_owner = v
        .get("required_pull_request_reviews")
        .and_then(|r| r.get("require_code_owner_reviews"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let strict = v
        .get("required_status_checks")
        .and_then(|r| r.get("strict"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let contexts = v
        .get("required_status_checks")
        .and_then(|r| r.get("contexts"))
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let enforce_admins = v
        .get("enforce_admins")
        .and_then(|r| r.get("enabled"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let allow_force_pushes = v
        .get("allow_force_pushes")
        .and_then(|r| r.get("enabled"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let allow_deletions = v
        .get("allow_deletions")
        .and_then(|r| r.get("enabled"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    BranchProtectionApi {
        required_reviews,
        dismiss_stale_reviews: dismiss_stale,
        require_code_owner_reviews: code_owner,
        strict_status_checks: strict,
        status_check_contexts: contexts,
        enforce_admins,
        allow_force_pushes,
        allow_deletions,
    }
}
