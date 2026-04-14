//! Repository client trait, API types, and pure comparison/apply logic.
//!
//! This module contains no I/O against external binaries and is suitable
//! for Miri testing. Production I/O implementations live in the `gh-sync`
//! binary crate.

// TODO: add per-item doc comments to satisfy `missing_docs` and `missing_errors_doc`
#![allow(missing_docs)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::must_use_candidate)]

use gh_sync_manifest::{BranchProtection, Ruleset};

// ---------------------------------------------------------------------------
// Submodules
// ---------------------------------------------------------------------------

mod apply;
mod compare;
mod parse;
mod print;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// Public re-exports
// ---------------------------------------------------------------------------

pub use apply::apply_changes;
pub use compare::compare;
pub use parse::{parse_branch_protection_api, parse_repo_api_data};
pub use print::print_preview;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

pub trait GhRepoClient {
    fn detect_repo(&self) -> anyhow::Result<String>;
    fn fetch_repo(&self, repo: &str) -> anyhow::Result<RepoApiData>;
    fn fetch_topics(&self, repo: &str) -> anyhow::Result<Vec<String>>;
    fn fetch_labels(&self, repo: &str) -> anyhow::Result<Vec<ApiLabel>>;
    fn fetch_actions_permissions(&self, repo: &str) -> anyhow::Result<ActionsPermissionsApi>;
    fn fetch_selected_actions(&self, repo: &str) -> anyhow::Result<SelectedActionsApi>;
    fn fetch_workflow_permissions(&self, repo: &str) -> anyhow::Result<WorkflowPermissionsApi>;
    fn fetch_rulesets(&self, repo: &str) -> anyhow::Result<Vec<LiveRuleset>>;
    fn fetch_ruleset_details(&self, repo: &str, id: u64) -> anyhow::Result<serde_json::Value>;
    fn fetch_branch_protection(
        &self,
        repo: &str,
        branch: &str,
    ) -> anyhow::Result<Option<BranchProtectionApi>>;
    fn list_protected_branches(&self, repo: &str) -> anyhow::Result<Vec<String>>;

    fn fetch_release_immutability(&self, repo: &str) -> anyhow::Result<Option<bool>>;
    fn put_release_immutability(&self, repo: &str, enabled: bool) -> anyhow::Result<()>;

    /// Fetch the fork-PR contributor approval policy via its dedicated endpoint.
    ///
    /// Returns `None` when the endpoint is not available (user-owned repos → 404,
    /// private repos → 422).
    fn fetch_fork_pr_approval(&self, repo: &str) -> anyhow::Result<Option<String>>;
    fn put_fork_pr_approval(&self, repo: &str, policy: &str) -> anyhow::Result<()>;

    fn patch_repo(&self, repo: &str, body: &serde_json::Value) -> anyhow::Result<()>;
    fn put_topics(&self, repo: &str, topics: &[String]) -> anyhow::Result<()>;
    fn create_label(
        &self,
        repo: &str,
        name: &str,
        color: &str,
        description: Option<&str>,
    ) -> anyhow::Result<()>;
    fn update_label(
        &self,
        repo: &str,
        name: &str,
        color: &str,
        description: Option<&str>,
    ) -> anyhow::Result<()>;
    fn delete_label(&self, repo: &str, name: &str) -> anyhow::Result<()>;
    fn put_actions_permissions(&self, repo: &str, body: &serde_json::Value) -> anyhow::Result<()>;
    fn put_selected_actions(&self, repo: &str, body: &serde_json::Value) -> anyhow::Result<()>;
    fn put_workflow_permissions(&self, repo: &str, body: &serde_json::Value) -> anyhow::Result<()>;
    fn create_ruleset(&self, repo: &str, body: &serde_json::Value) -> anyhow::Result<()>;
    fn update_ruleset(&self, repo: &str, id: u64, body: &serde_json::Value) -> anyhow::Result<()>;
    fn delete_ruleset(&self, repo: &str, id: u64) -> anyhow::Result<()>;
    fn put_branch_protection(
        &self,
        repo: &str,
        branch: &str,
        body: &serde_json::Value,
    ) -> anyhow::Result<()>;
    fn delete_branch_protection(&self, repo: &str, branch: &str) -> anyhow::Result<()>;
    fn resolve_team_id(&self, org: &str, team_slug: &str) -> anyhow::Result<u64>;
    fn resolve_app_id(&self, app_slug: &str) -> anyhow::Result<u64>;
    fn resolve_org_custom_role_id(&self, org: &str, role_name: &str) -> anyhow::Result<u64>;
}

// ---------------------------------------------------------------------------
// API response types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
#[allow(clippy::module_name_repetitions)]
pub struct RepoApiData {
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub visibility: String,
    pub archived: bool,
    pub has_issues: bool,
    pub has_projects: bool,
    pub has_wiki: bool,
    pub has_discussions: bool,
    pub delete_branch_on_merge: bool,
    pub allow_merge_commit: bool,
    pub allow_squash_merge: bool,
    pub allow_rebase_merge: bool,
    pub allow_auto_merge: bool,
    pub allow_update_branch: bool,
    pub web_commit_signoff_required: bool,
    pub squash_merge_commit_title: Option<String>,
    pub squash_merge_commit_message: Option<String>,
    pub merge_commit_title: Option<String>,
    pub merge_commit_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiLabel {
    pub name: String,
    pub color: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ActionsPermissionsApi {
    pub enabled: bool,
    pub allowed_actions: Option<String>,
    pub sha_pinning_required: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct SelectedActionsApi {
    pub github_owned_allowed: Option<bool>,
    pub patterns_allowed: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct WorkflowPermissionsApi {
    pub default_workflow_permissions: Option<String>,
    pub can_approve_pull_request_reviews: bool,
}

#[derive(Debug, Clone)]
pub struct LiveRuleset {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct BranchProtectionApi {
    pub required_reviews: Option<u32>,
    pub dismiss_stale_reviews: bool,
    pub require_code_owner_reviews: bool,
    pub strict_status_checks: bool,
    pub status_check_contexts: Vec<String>,
    pub enforce_admins: bool,
    pub allow_force_pushes: bool,
    pub allow_deletions: bool,
}

// ---------------------------------------------------------------------------
// Change descriptors
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum SpecChange {
    FieldChanged {
        field: String,
        old: String,
        new: String,
    },
    FieldOk {
        field: String,
        value: String,
    },
    LabelAdd {
        name: String,
        color: String,
        description: Option<String>,
    },
    LabelUpdate {
        name: String,
        old_color: String,
        old_description: Option<String>,
        new_color: String,
        new_description: Option<String>,
    },
    LabelDelete {
        name: String,
    },
    LabelOk {
        name: String,
    },
    RulesetAdd {
        name: String,
        spec: Box<Ruleset>,
    },
    RulesetUpdate {
        id: u64,
        name: String,
        spec: Box<Ruleset>,
        /// Live ruleset JSON from the API; `None` when detail fetch failed.
        live: Option<Box<serde_json::Value>>,
    },
    RulesetOk {
        id: u64,
        name: String,
    },
    RulesetDelete {
        id: u64,
        name: String,
    },
    BranchProtectionAdd {
        spec: Box<BranchProtection>,
    },
    BranchProtectionUpdate {
        spec: Box<BranchProtection>,
    },
    BranchProtectionRemove {
        pattern: String,
    },
    BranchProtectionOk {
        pattern: String,
    },
}
