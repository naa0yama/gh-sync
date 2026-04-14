#![allow(missing_debug_implementations)]
#![allow(clippy::missing_docs_in_private_items)]
#![allow(missing_docs)]
#![allow(clippy::wildcard_imports)]
#![allow(clippy::must_use_candidate)]

use std::cell::RefCell;
use std::collections::HashMap;

use super::*;

pub struct MockRepoClient {
    pub repo_name: String,
    pub repo_data: RepoApiData,
    pub topics: Vec<String>,
    pub labels: Vec<ApiLabel>,
    pub actions_permissions: ActionsPermissionsApi,
    pub selected_actions: SelectedActionsApi,
    pub workflow_permissions: WorkflowPermissionsApi,
    pub rulesets: Vec<LiveRuleset>,
    /// Map from ruleset id to the full JSON returned by `GET rulesets/{id}`.
    /// Missing entries cause `fetch_ruleset_details` to return an error
    /// (conservative: treated as update by `compare_rulesets`).
    pub ruleset_details: HashMap<u64, serde_json::Value>,
    /// Map from branch name to protection state (None = 404).
    pub branch_protections: HashMap<String, Option<BranchProtectionApi>>,
    /// Protected branch names (for `list_protected_branches`).
    pub protected_branches: Vec<String>,
    /// Team slug → id
    pub team_ids: HashMap<String, u64>,
    /// App slug → id
    pub app_ids: HashMap<String, u64>,
    /// Custom role name → id
    pub custom_role_ids: HashMap<String, u64>,

    // Recorded calls
    pub detect_repo_calls: std::cell::Cell<usize>,
    pub applied_patches: RefCell<Vec<serde_json::Value>>,
    pub applied_topics: RefCell<Option<Vec<String>>>,
    pub created_labels: RefCell<Vec<String>>,
    pub updated_labels: RefCell<Vec<String>>,
    pub deleted_labels: RefCell<Vec<String>>,
    pub created_rulesets: RefCell<Vec<serde_json::Value>>,
    pub updated_rulesets: RefCell<Vec<(u64, serde_json::Value)>>,
    pub deleted_rulesets: RefCell<Vec<u64>>,
    pub put_branch_protections: RefCell<Vec<(String, serde_json::Value)>>,
    pub deleted_branch_protections: RefCell<Vec<String>>,
    pub put_workflow_permissions_body: RefCell<Option<serde_json::Value>>,
    pub release_immutability: Option<bool>,
    pub put_release_immutability_calls: RefCell<Vec<bool>>,
    pub fork_pr_approval: Option<String>,
}

impl MockRepoClient {
    pub fn new(repo_name: &str) -> Self {
        Self {
            repo_name: repo_name.to_owned(),
            repo_data: RepoApiData {
                description: None,
                homepage: None,
                visibility: String::from("public"),
                archived: false,
                has_issues: true,
                has_projects: true,
                has_wiki: true,
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
            },
            topics: Vec::new(),
            labels: Vec::new(),
            actions_permissions: ActionsPermissionsApi {
                enabled: true,
                allowed_actions: Some(String::from("all")),
                sha_pinning_required: None,
            },
            selected_actions: SelectedActionsApi {
                github_owned_allowed: None,
                patterns_allowed: None,
            },
            workflow_permissions: WorkflowPermissionsApi {
                default_workflow_permissions: Some(String::from("read")),
                can_approve_pull_request_reviews: false,
            },
            rulesets: Vec::new(),
            ruleset_details: HashMap::new(),
            branch_protections: HashMap::new(),
            protected_branches: Vec::new(),
            team_ids: HashMap::new(),
            app_ids: HashMap::new(),
            custom_role_ids: HashMap::new(),
            detect_repo_calls: std::cell::Cell::new(0),
            applied_patches: RefCell::new(Vec::new()),
            applied_topics: RefCell::new(None),
            created_labels: RefCell::new(Vec::new()),
            updated_labels: RefCell::new(Vec::new()),
            deleted_labels: RefCell::new(Vec::new()),
            created_rulesets: RefCell::new(Vec::new()),
            updated_rulesets: RefCell::new(Vec::new()),
            deleted_rulesets: RefCell::new(Vec::new()),
            put_branch_protections: RefCell::new(Vec::new()),
            deleted_branch_protections: RefCell::new(Vec::new()),
            put_workflow_permissions_body: RefCell::new(None),
            release_immutability: None,
            put_release_immutability_calls: RefCell::new(Vec::new()),
            fork_pr_approval: None,
        }
    }
}

impl GhRepoClient for MockRepoClient {
    fn detect_repo(&self) -> anyhow::Result<String> {
        self.detect_repo_calls
            .set(self.detect_repo_calls.get().saturating_add(1));
        Ok(self.repo_name.clone())
    }

    fn fetch_repo(&self, _repo: &str) -> anyhow::Result<RepoApiData> {
        Ok(self.repo_data.clone())
    }

    fn fetch_topics(&self, _repo: &str) -> anyhow::Result<Vec<String>> {
        Ok(self.topics.clone())
    }

    fn fetch_labels(&self, _repo: &str) -> anyhow::Result<Vec<ApiLabel>> {
        Ok(self.labels.clone())
    }

    fn fetch_actions_permissions(&self, _repo: &str) -> anyhow::Result<ActionsPermissionsApi> {
        Ok(self.actions_permissions.clone())
    }

    fn fetch_selected_actions(&self, _repo: &str) -> anyhow::Result<SelectedActionsApi> {
        Ok(self.selected_actions.clone())
    }

    fn fetch_workflow_permissions(&self, _repo: &str) -> anyhow::Result<WorkflowPermissionsApi> {
        Ok(self.workflow_permissions.clone())
    }

    fn fetch_rulesets(&self, _repo: &str) -> anyhow::Result<Vec<LiveRuleset>> {
        Ok(self.rulesets.clone())
    }

    fn fetch_ruleset_details(&self, _repo: &str, id: u64) -> anyhow::Result<serde_json::Value> {
        self.ruleset_details
            .get(&id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("ruleset {id} not in mock"))
    }

    fn fetch_branch_protection(
        &self,
        _repo: &str,
        branch: &str,
    ) -> anyhow::Result<Option<BranchProtectionApi>> {
        Ok(self.branch_protections.get(branch).cloned().unwrap_or(None))
    }

    fn list_protected_branches(&self, _repo: &str) -> anyhow::Result<Vec<String>> {
        Ok(self.protected_branches.clone())
    }

    fn fetch_release_immutability(&self, _repo: &str) -> anyhow::Result<Option<bool>> {
        Ok(self.release_immutability)
    }

    fn put_release_immutability(&self, _repo: &str, enabled: bool) -> anyhow::Result<()> {
        self.put_release_immutability_calls
            .borrow_mut()
            .push(enabled);
        Ok(())
    }

    fn fetch_fork_pr_approval(&self, _repo: &str) -> anyhow::Result<Option<String>> {
        Ok(self.fork_pr_approval.clone())
    }

    fn put_fork_pr_approval(&self, _repo: &str, _policy: &str) -> anyhow::Result<()> {
        Ok(())
    }

    fn patch_repo(&self, _repo: &str, body: &serde_json::Value) -> anyhow::Result<()> {
        self.applied_patches.borrow_mut().push(body.clone());
        Ok(())
    }

    fn put_topics(&self, _repo: &str, topics: &[String]) -> anyhow::Result<()> {
        *self.applied_topics.borrow_mut() = Some(topics.to_vec());
        Ok(())
    }

    fn create_label(
        &self,
        _repo: &str,
        name: &str,
        _color: &str,
        _description: Option<&str>,
    ) -> anyhow::Result<()> {
        self.created_labels.borrow_mut().push(name.to_owned());
        Ok(())
    }

    fn update_label(
        &self,
        _repo: &str,
        name: &str,
        _color: &str,
        _description: Option<&str>,
    ) -> anyhow::Result<()> {
        self.updated_labels.borrow_mut().push(name.to_owned());
        Ok(())
    }

    fn delete_label(&self, _repo: &str, name: &str) -> anyhow::Result<()> {
        self.deleted_labels.borrow_mut().push(name.to_owned());
        Ok(())
    }

    fn put_actions_permissions(
        &self,
        _repo: &str,
        _body: &serde_json::Value,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn put_selected_actions(&self, _repo: &str, _body: &serde_json::Value) -> anyhow::Result<()> {
        Ok(())
    }

    fn put_workflow_permissions(
        &self,
        _repo: &str,
        body: &serde_json::Value,
    ) -> anyhow::Result<()> {
        *self.put_workflow_permissions_body.borrow_mut() = Some(body.clone());
        Ok(())
    }

    fn create_ruleset(&self, _repo: &str, body: &serde_json::Value) -> anyhow::Result<()> {
        self.created_rulesets.borrow_mut().push(body.clone());
        Ok(())
    }

    fn update_ruleset(&self, _repo: &str, id: u64, body: &serde_json::Value) -> anyhow::Result<()> {
        self.updated_rulesets.borrow_mut().push((id, body.clone()));
        Ok(())
    }

    fn delete_ruleset(&self, _repo: &str, id: u64) -> anyhow::Result<()> {
        self.deleted_rulesets.borrow_mut().push(id);
        Ok(())
    }

    fn put_branch_protection(
        &self,
        _repo: &str,
        branch: &str,
        body: &serde_json::Value,
    ) -> anyhow::Result<()> {
        self.put_branch_protections
            .borrow_mut()
            .push((branch.to_owned(), body.clone()));
        Ok(())
    }

    fn delete_branch_protection(&self, _repo: &str, branch: &str) -> anyhow::Result<()> {
        self.deleted_branch_protections
            .borrow_mut()
            .push(branch.to_owned());
        Ok(())
    }

    fn resolve_team_id(&self, _org: &str, team_slug: &str) -> anyhow::Result<u64> {
        self.team_ids
            .get(team_slug)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("team '{team_slug}' not in mock"))
    }

    fn resolve_app_id(&self, app_slug: &str) -> anyhow::Result<u64> {
        self.app_ids
            .get(app_slug)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("app '{app_slug}' not in mock"))
    }

    fn resolve_org_custom_role_id(&self, _org: &str, role_name: &str) -> anyhow::Result<u64> {
        self.custom_role_ids
            .get(role_name)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("custom role '{role_name}' not in mock"))
    }
}
