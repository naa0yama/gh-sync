//! `sync repo` — I/O implementation and entry point.
//!
//! Pure logic (trait, types, compare, print, apply) lives in `gh_sync_engine::repo`.
//! This module contains only the `gh` CLI implementation and the top-level entry point.

const BRANCH_NOT_PROTECTED: &str = "Branch not protected";

use std::io::{self, IsTerminal as _, Write};
use std::process::ExitCode;

use anyhow::Context as _;
#[allow(clippy::module_name_repetitions)]
pub use gh_sync_engine::repo::{
    ActionsPermissionsApi, ApiLabel, BranchProtectionApi, GhRepoClient, LiveRuleset, RepoApiData,
    SelectedActionsApi, SpecChange, WorkflowPermissionsApi, apply_changes, compare,
    parse_branch_protection_api, parse_repo_api_data, print_preview,
};
use gh_sync_manifest::{Manifest, Spec};

use crate::sync::manifest;
use crate::sync::runner::{GhRunner, SystemGhRunner};

// ---------------------------------------------------------------------------
// Production GhRepoClient
// ---------------------------------------------------------------------------

/// `gh`-backed implementation of [`GhRepoClient`].
///
/// Generic over [`GhRunner`] so that unit tests can inject a [`MockGhRunner`]
/// that returns canned responses without spawning a real process.
pub struct GhRepoClientImpl<R: GhRunner = SystemGhRunner> {
    runner: R,
}

impl GhRepoClientImpl<SystemGhRunner> {
    /// Create a new instance backed by the real `gh` CLI.
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub const fn new() -> Self {
        Self {
            runner: SystemGhRunner,
        }
    }
}

impl Default for GhRepoClientImpl<SystemGhRunner> {
    #[cfg_attr(coverage_nightly, coverage(off))]
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Internal helpers (routing through GhRunner)
// ---------------------------------------------------------------------------

impl<R: GhRunner> GhRepoClientImpl<R> {
    /// `gh api <url>` → raw bytes.
    fn gh_api_get(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        let out = self
            .runner
            .run(&["api", url], None)
            .with_context(|| format!("failed to spawn `gh api {url}`"))?;
        if !out.success() {
            anyhow::bail!(
                "`gh api GET {url}` failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(out.stdout)
    }

    /// `gh api <url>` → parsed JSON value.
    fn gh_api_get_json(&self, url: &str) -> anyhow::Result<serde_json::Value> {
        let bytes = self.gh_api_get(url)?;
        serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse JSON from `gh api {url}`"))
    }

    /// `gh api -X <method> <url> --input -` with JSON body written to stdin.
    fn gh_api_write(
        &self,
        method: &str,
        url: &str,
        body: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let json = serde_json::to_string(body).context("failed to serialize body")?;
        let out = self
            .runner
            .run(
                &["api", "-X", method, url, "--input", "-"],
                Some(json.as_bytes()),
            )
            .with_context(|| format!("failed to spawn `gh api {method} {url}`"))?;
        if !out.success() {
            anyhow::bail!(
                "`gh api {method} {url}` failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// GhRepoClient implementation
// ---------------------------------------------------------------------------

impl<R: GhRunner> GhRepoClient for GhRepoClientImpl<R> {
    fn detect_repo(&self) -> anyhow::Result<String> {
        let out = self
            .runner
            .run(
                &[
                    "repo",
                    "view",
                    "--json",
                    "nameWithOwner",
                    "-q",
                    ".nameWithOwner",
                ],
                None,
            )
            .context("failed to spawn `gh repo view`")?;
        if !out.success() {
            anyhow::bail!(
                "`gh repo view` failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    fn fetch_repo(&self, repo: &str) -> anyhow::Result<RepoApiData> {
        let v = self.gh_api_get_json(&format!("repos/{repo}"))?;
        Ok(parse_repo_api_data(&v))
    }

    fn fetch_topics(&self, repo: &str) -> anyhow::Result<Vec<String>> {
        let v = self.gh_api_get_json(&format!("repos/{repo}/topics"))?;
        Ok(v.get("names")
            .and_then(|n| n.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default())
    }

    fn fetch_labels(&self, repo: &str) -> anyhow::Result<Vec<ApiLabel>> {
        let out = self
            .runner
            .run(
                &[
                    "label",
                    "list",
                    "--repo",
                    repo,
                    "--limit",
                    "1000",
                    "--json",
                    "name,color,description",
                ],
                None,
            )
            .with_context(|| format!("failed to spawn `gh label list --repo {repo}`"))?;
        if !out.success() {
            anyhow::bail!(
                "`gh label list` failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        let items: Vec<serde_json::Value> =
            serde_json::from_slice(&out.stdout).context("failed to parse labels JSON")?;
        Ok(items
            .iter()
            .map(|v| ApiLabel {
                name: v["name"].as_str().unwrap_or("").to_owned(),
                color: v["color"].as_str().unwrap_or("").to_owned(),
                description: v["description"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
            })
            .collect())
    }

    fn fetch_actions_permissions(&self, repo: &str) -> anyhow::Result<ActionsPermissionsApi> {
        let v = self.gh_api_get_json(&format!("repos/{repo}/actions/permissions"))?;
        Ok(ActionsPermissionsApi {
            enabled: v
                .get("enabled")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
            allowed_actions: v
                .get("allowed_actions")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            sha_pinning_required: v
                .get("sha_pinning_required")
                .and_then(serde_json::Value::as_bool),
        })
    }

    fn fetch_selected_actions(&self, repo: &str) -> anyhow::Result<SelectedActionsApi> {
        let v = self.gh_api_get_json(&format!(
            "repos/{repo}/actions/permissions/selected-actions"
        ))?;
        Ok(SelectedActionsApi {
            github_owned_allowed: v
                .get("github_owned_allowed")
                .and_then(serde_json::Value::as_bool),
            patterns_allowed: v
                .get("patterns_allowed")
                .and_then(serde_json::Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(str::to_owned))
                        .collect()
                }),
        })
    }

    fn fetch_workflow_permissions(&self, repo: &str) -> anyhow::Result<WorkflowPermissionsApi> {
        let v = self.gh_api_get_json(&format!("repos/{repo}/actions/permissions/workflow"))?;
        Ok(WorkflowPermissionsApi {
            default_workflow_permissions: v
                .get("default_workflow_permissions")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            can_approve_pull_request_reviews: v
                .get("can_approve_pull_request_reviews")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        })
    }

    fn fetch_ruleset_details(&self, repo: &str, id: u64) -> anyhow::Result<serde_json::Value> {
        self.gh_api_get_json(&format!("repos/{repo}/rulesets/{id}"))
    }

    fn fetch_rulesets(&self, repo: &str) -> anyhow::Result<Vec<LiveRuleset>> {
        let arr = self.gh_api_get_json(&format!("repos/{repo}/rulesets?per_page=100"))?;
        let arr = arr
            .as_array()
            .context("rulesets response is not an array")?;
        Ok(arr
            .iter()
            .filter_map(|v| {
                let id = v["id"].as_u64()?;
                let name = v["name"].as_str()?.to_owned();
                Some(LiveRuleset { id, name })
            })
            .collect())
    }

    fn fetch_branch_protection(
        &self,
        repo: &str,
        branch: &str,
    ) -> anyhow::Result<Option<BranchProtectionApi>> {
        let out = self
            .runner
            .run(
                &["api", &format!("repos/{repo}/branches/{branch}/protection")],
                None,
            )
            .with_context(|| {
                format!("failed to spawn `gh api` for branch protection {repo}/{branch}")
            })?;
        if out.exit_code == Some(1) {
            let body = String::from_utf8_lossy(&out.stderr);
            if body.contains("404") || body.contains(BRANCH_NOT_PROTECTED) {
                return Ok(None);
            }
        }
        if !out.success() {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout)
                && v.get("message").and_then(serde_json::Value::as_str)
                    == Some(BRANCH_NOT_PROTECTED)
            {
                return Ok(None);
            }
            anyhow::bail!(
                "`gh api` branch protection failed: {}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
        let v: serde_json::Value =
            serde_json::from_slice(&out.stdout).context("failed to parse branch protection")?;
        if v.get("message").and_then(serde_json::Value::as_str) == Some(BRANCH_NOT_PROTECTED) {
            return Ok(None);
        }
        Ok(Some(parse_branch_protection_api(&v)))
    }

    fn list_protected_branches(&self, repo: &str) -> anyhow::Result<Vec<String>> {
        let arr = self.gh_api_get_json(&format!(
            "repos/{repo}/branches?protected=true&per_page=100"
        ))?;
        Ok(arr
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default())
    }

    fn fetch_release_immutability(&self, repo: &str) -> anyhow::Result<Option<bool>> {
        let out = self
            .runner
            .run(&["api", &format!("repos/{repo}/immutable-releases")], None)
            .with_context(|| format!("failed to spawn `gh api` for immutable-releases {repo}"))?;
        if !out.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            // 404/403 means the endpoint is not available (e.g. GHES without support).
            if stderr.contains("404") || stderr.contains("403") {
                return Ok(None);
            }
            anyhow::bail!("`gh api GET immutable-releases` failed: {stderr}");
        }
        let v: serde_json::Value = serde_json::from_slice(&out.stdout)
            .context("failed to parse immutable-releases JSON")?;
        Ok(v.get("enabled").and_then(serde_json::Value::as_bool))
    }

    fn put_release_immutability(&self, repo: &str, enabled: bool) -> anyhow::Result<()> {
        let method = if enabled { "PUT" } else { "DELETE" };
        let out = self
            .runner
            .run(
                &[
                    "api",
                    "-X",
                    method,
                    &format!("repos/{repo}/immutable-releases"),
                ],
                None,
            )
            .with_context(|| {
                format!("failed to spawn `gh api {method} immutable-releases {repo}`")
            })?;
        if !out.success() {
            anyhow::bail!(
                "`gh api {method} immutable-releases` failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(())
    }

    fn fetch_fork_pr_approval(&self, repo: &str) -> anyhow::Result<Option<String>> {
        let out = self
            .runner
            .run(
                &[
                    "api",
                    &format!("repos/{repo}/actions/permissions/fork-pr-contributor-approval"),
                ],
                None,
            )
            .with_context(|| format!("failed to spawn `gh api` for fork-pr-approval {repo}"))?;
        if !out.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            // 404 = user-owned repo, 422 = private repo — both are expected
            if stderr.contains("404") || stderr.contains("422") {
                return Ok(None);
            }
            anyhow::bail!("`gh api GET fork-pr-contributor-approval` failed: {stderr}");
        }
        let v: serde_json::Value = serde_json::from_slice(&out.stdout)
            .context("failed to parse fork-pr-contributor-approval JSON")?;
        Ok(v.get("approval_policy")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned))
    }

    fn put_fork_pr_approval(&self, repo: &str, policy: &str) -> anyhow::Result<()> {
        let body = serde_json::json!({ "approval_policy": policy });
        self.gh_api_write(
            "PUT",
            &format!("repos/{repo}/actions/permissions/fork-pr-contributor-approval"),
            &body,
        )
    }

    fn patch_repo(&self, repo: &str, body: &serde_json::Value) -> anyhow::Result<()> {
        self.gh_api_write("PATCH", &format!("repos/{repo}"), body)
    }

    fn put_topics(&self, repo: &str, topics: &[String]) -> anyhow::Result<()> {
        let body = serde_json::json!({ "names": topics });
        self.gh_api_write("PUT", &format!("repos/{repo}/topics"), &body)
    }

    fn create_label(
        &self,
        repo: &str,
        name: &str,
        color: &str,
        description: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut args = vec!["label", "create", name, "--repo", repo, "--color", color];
        if let Some(desc) = description {
            args.extend_from_slice(&["--description", desc]);
        }
        let out = self
            .runner
            .run(&args, None)
            .with_context(|| format!("failed to spawn `gh label create {name} --repo {repo}`"))?;
        if !out.success() {
            anyhow::bail!(
                "`gh label create` failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(())
    }

    fn update_label(
        &self,
        repo: &str,
        name: &str,
        color: &str,
        description: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut args = vec!["label", "edit", name, "--repo", repo, "--color", color];
        if let Some(desc) = description {
            args.extend_from_slice(&["--description", desc]);
        }
        let out = self
            .runner
            .run(&args, None)
            .with_context(|| format!("failed to spawn `gh label edit {name} --repo {repo}`"))?;
        if !out.success() {
            anyhow::bail!(
                "`gh label edit` failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(())
    }

    fn delete_label(&self, repo: &str, name: &str) -> anyhow::Result<()> {
        let out = self
            .runner
            .run(
                &["label", "delete", name, "--repo", repo, "--confirm"],
                None,
            )
            .with_context(|| format!("failed to spawn `gh label delete {name} --repo {repo}`"))?;
        if !out.success() {
            anyhow::bail!(
                "`gh label delete` failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(())
    }

    fn put_actions_permissions(&self, repo: &str, body: &serde_json::Value) -> anyhow::Result<()> {
        self.gh_api_write("PUT", &format!("repos/{repo}/actions/permissions"), body)
    }

    fn put_selected_actions(&self, repo: &str, body: &serde_json::Value) -> anyhow::Result<()> {
        self.gh_api_write(
            "PUT",
            &format!("repos/{repo}/actions/permissions/selected-actions"),
            body,
        )
    }

    fn put_workflow_permissions(&self, repo: &str, body: &serde_json::Value) -> anyhow::Result<()> {
        self.gh_api_write(
            "PUT",
            &format!("repos/{repo}/actions/permissions/workflow"),
            body,
        )
    }

    fn create_ruleset(&self, repo: &str, body: &serde_json::Value) -> anyhow::Result<()> {
        self.gh_api_write("POST", &format!("repos/{repo}/rulesets"), body)
    }

    fn update_ruleset(&self, repo: &str, id: u64, body: &serde_json::Value) -> anyhow::Result<()> {
        self.gh_api_write("PUT", &format!("repos/{repo}/rulesets/{id}"), body)
    }

    fn delete_ruleset(&self, repo: &str, id: u64) -> anyhow::Result<()> {
        let out = self
            .runner
            .run(
                &[
                    "api",
                    "-X",
                    "DELETE",
                    &format!("repos/{repo}/rulesets/{id}"),
                ],
                None,
            )
            .with_context(|| format!("failed to spawn `gh api DELETE rulesets/{id}` for {repo}"))?;
        if !out.success() {
            anyhow::bail!(
                "`gh api DELETE rulesets/{id}` failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(())
    }

    fn put_branch_protection(
        &self,
        repo: &str,
        branch: &str,
        body: &serde_json::Value,
    ) -> anyhow::Result<()> {
        self.gh_api_write(
            "PUT",
            &format!("repos/{repo}/branches/{branch}/protection"),
            body,
        )
    }

    fn delete_branch_protection(&self, repo: &str, branch: &str) -> anyhow::Result<()> {
        let out = self
            .runner
            .run(
                &[
                    "api",
                    "-X",
                    "DELETE",
                    &format!("repos/{repo}/branches/{branch}/protection"),
                ],
                None,
            )
            .with_context(|| {
                format!("failed to spawn `gh api DELETE branch protection` for {repo}/{branch}")
            })?;
        if !out.success() {
            anyhow::bail!(
                "`gh api DELETE branch protection` failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(())
    }

    fn resolve_team_id(&self, org: &str, team_slug: &str) -> anyhow::Result<u64> {
        let v = self.gh_api_get_json(&format!("orgs/{org}/teams/{team_slug}"))?;
        v.get("id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| anyhow::anyhow!("team '{team_slug}' has no id field"))
    }

    fn resolve_app_id(&self, app_slug: &str) -> anyhow::Result<u64> {
        let v = self.gh_api_get_json(&format!("apps/{app_slug}"))?;
        v.get("id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| anyhow::anyhow!("app '{app_slug}' has no id field"))
    }

    fn resolve_org_custom_role_id(&self, org: &str, role_name: &str) -> anyhow::Result<u64> {
        let v = self.gh_api_get_json(&format!("orgs/{org}/custom-repository-roles"))?;
        let empty = vec![];
        let roles = v
            .get("custom_roles")
            .and_then(serde_json::Value::as_array)
            .or_else(|| v.as_array())
            .unwrap_or(&empty);
        for role in roles {
            if role.get("name").and_then(serde_json::Value::as_str) == Some(role_name) {
                return role
                    .get("id")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| anyhow::anyhow!("custom role '{role_name}' has no id"));
            }
        }
        anyhow::bail!("custom role '{role_name}' not found in org '{org}'")
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[cfg_attr(coverage_nightly, coverage(off))]
pub fn execute(args: &super::cli::SyncRepoArgs) -> ExitCode {
    let mut stdout = io::stdout();
    execute_inner(args, &GhRepoClientImpl::new(), &mut stdout)
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn execute_inner(
    args: &super::cli::SyncRepoArgs,
    client: &dyn GhRepoClient,
    w: &mut dyn Write,
) -> ExitCode {
    let manifest = match Manifest::load(&args.manifest) {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("failed to load manifest: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Validate schema and local references before making any API calls.
    // A misconfigured manifest must be caught early to prevent partial apply.
    let repo_root = args
        .manifest
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .unwrap_or_else(|| std::path::Path::new("."));
    if let Err(e) = manifest::validate_schema(&manifest) {
        tracing::error!("manifest validation failed: {e}");
        return ExitCode::FAILURE;
    }
    if let Err(e) = manifest::validate_references(&manifest, repo_root) {
        tracing::error!("manifest validation failed: {e}");
        return ExitCode::FAILURE;
    }

    let Some(spec) = &manifest.spec else {
        let _ = writeln!(w, "no spec: section in manifest — nothing to do");
        return ExitCode::SUCCESS;
    };

    let repo = match client.detect_repo() {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("failed to detect repository: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = writeln!(w, "  fetching repository settings...") {
        tracing::error!("output error: {e}");
        return ExitCode::FAILURE;
    }
    let changes = match compare(spec, &repo, client) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("failed to fetch repository state: {e}");
            return ExitCode::FAILURE;
        }
    };

    let (_, has_actions) = match print_preview(w, &changes, &repo) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("output error: {e}");
            return ExitCode::FAILURE;
        }
    };

    if args.ci_check {
        return if has_actions {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        };
    }

    if !has_actions {
        return ExitCode::SUCCESS;
    }

    if args.dry_run {
        return ExitCode::SUCCESS;
    }

    if args.yes {
        return run_apply(&changes, spec, &repo, client);
    }

    if io::stdin().is_terminal() {
        let confirmed = dialoguer::Confirm::new()
            .with_prompt("Apply these changes?")
            .default(false)
            .interact()
            .unwrap_or(false);

        if confirmed {
            return run_apply(&changes, spec, &repo, client);
        }

        tracing::info!("aborted — no changes were applied");
        return ExitCode::SUCCESS;
    }

    tracing::error!(
        "changes detected but stdin is not a TTY; use --yes to apply or --dry-run to suppress this error"
    );
    ExitCode::FAILURE
}

fn run_apply(
    changes: &[SpecChange],
    spec: &Spec,
    repo: &str,
    client: &dyn GhRepoClient,
) -> ExitCode {
    tracing::info!("applying changes to {repo}");
    if let Err(e) = apply_changes(changes, spec, repo, client) {
        tracing::error!("failed to apply changes: {e}");
        return ExitCode::FAILURE;
    }
    tracing::info!("all changes applied successfully");
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::indexing_slicing)]

    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;
    use crate::sync::runner::{GhOutput, GhRunner};
    use gh_sync_engine::repo::testing::MockRepoClient;

    // ------------------------------------------------------------------
    // MockGhRunner — injects canned responses for GhRepoClientImpl tests
    // ------------------------------------------------------------------

    struct MockGhRunner {
        /// Queue of responses returned in FIFO order.
        queue: Mutex<VecDeque<GhOutput>>,
        /// All invocations recorded as `(args, stdin)`.
        #[allow(clippy::type_complexity)]
        calls: Mutex<Vec<(Vec<String>, Option<Vec<u8>>)>>,
    }

    impl MockGhRunner {
        fn new(responses: Vec<GhOutput>) -> Self {
            Self {
                queue: Mutex::new(responses.into()),
                calls: Mutex::new(Vec::new()),
            }
        }

        /// Shorthand: single success response with given stdout bytes.
        fn ok(stdout: impl Into<Vec<u8>>) -> Self {
            Self::new(vec![GhOutput {
                exit_code: Some(0),
                stdout: stdout.into(),
                stderr: vec![],
            }])
        }

        /// Shorthand: single failure response with given stderr text.
        fn err(stderr: impl Into<Vec<u8>>) -> Self {
            Self::new(vec![GhOutput {
                exit_code: Some(1),
                stdout: vec![],
                stderr: stderr.into(),
            }])
        }

        /// All args recorded so far (first call's args are `calls()[0]`).
        fn calls(&self) -> Vec<Vec<String>> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|(a, _)| a.clone())
                .collect()
        }

        /// Stdin bytes of the nth call.
        fn stdin_of(&self, n: usize) -> Option<Vec<u8>> {
            self.calls
                .lock()
                .unwrap()
                .get(n)
                .and_then(|(_, s)| s.clone())
        }
    }

    impl GhRunner for MockGhRunner {
        fn run(&self, args: &[&str], stdin: Option<&[u8]>) -> anyhow::Result<GhOutput> {
            self.calls.lock().unwrap().push((
                args.iter().map(|s| (*s).to_owned()).collect(),
                stdin.map(<[u8]>::to_vec),
            ));
            self.queue
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("MockGhRunner: no more responses queued"))
        }
    }

    fn client(runner: MockGhRunner) -> GhRepoClientImpl<MockGhRunner> {
        GhRepoClientImpl { runner }
    }

    fn success_json(v: &serde_json::Value) -> GhOutput {
        GhOutput {
            exit_code: Some(0),
            stdout: serde_json::to_vec(v).unwrap(),
            stderr: vec![],
        }
    }

    fn failure(code: i32, stderr: &str) -> GhOutput {
        GhOutput {
            exit_code: Some(code),
            stdout: vec![],
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    // ------------------------------------------------------------------
    // detect_repo
    // ------------------------------------------------------------------

    #[test]
    fn detect_repo_returns_trimmed_name() {
        // Arrange
        let c = client(MockGhRunner::ok(b"owner/repo\n".to_vec()));

        // Act
        let result = c.detect_repo().unwrap();

        // Assert
        assert_eq!(result, "owner/repo");
        let calls = c.runner.calls();
        assert_eq!(
            calls[0],
            [
                "repo",
                "view",
                "--json",
                "nameWithOwner",
                "-q",
                ".nameWithOwner"
            ]
        );
    }

    #[test]
    fn detect_repo_propagates_error() {
        // Arrange
        let c = client(MockGhRunner::err(b"not authenticated".to_vec()));

        // Act
        let err = c.detect_repo().unwrap_err();

        // Assert
        assert!(err.to_string().contains("`gh repo view` failed"));
    }

    // ------------------------------------------------------------------
    // fetch_repo
    // ------------------------------------------------------------------

    #[test]
    fn fetch_repo_calls_correct_endpoint() {
        // Arrange
        let json = serde_json::json!({
            "name": "repo", "full_name": "owner/repo", "description": null,
            "private": false, "visibility": "public", "default_branch": "main",
            "topics": [], "has_issues": true, "has_projects": true,
            "has_wiki": true, "has_downloads": true, "archived": false,
            "allow_squash_merge": true, "allow_merge_commit": true, "allow_rebase_merge": true,
            "allow_auto_merge": false, "delete_branch_on_merge": false,
            "allow_update_branch": false, "web_commit_signoff_required": false,
            "squash_merge_commit_title": null, "squash_merge_commit_message": null,
            "merge_commit_title": null, "merge_commit_message": null
        });
        let c = client(MockGhRunner::new(vec![success_json(&json)]));

        // Act
        c.fetch_repo("owner/repo").unwrap();

        // Assert
        assert_eq!(c.runner.calls()[0], ["api", "repos/owner/repo"]);
    }

    #[test]
    fn fetch_repo_propagates_api_error() {
        // Arrange
        let c = client(MockGhRunner::err(b"500 error".to_vec()));

        // Act
        let err = c.fetch_repo("owner/repo").unwrap_err();

        // Assert
        assert!(err.to_string().contains("`gh api GET"));
    }

    // ------------------------------------------------------------------
    // fetch_topics
    // ------------------------------------------------------------------

    #[test]
    fn fetch_topics_parses_names_array() {
        // Arrange
        let json = serde_json::json!({"names": ["rust", "cli"]});
        let c = client(MockGhRunner::new(vec![success_json(&json)]));

        // Act
        let topics = c.fetch_topics("owner/repo").unwrap();

        // Assert
        assert_eq!(topics, ["rust", "cli"]);
        assert_eq!(c.runner.calls()[0], ["api", "repos/owner/repo/topics"]);
    }

    #[test]
    fn fetch_topics_returns_empty_on_missing_names() {
        // Arrange
        let c = client(MockGhRunner::new(vec![success_json(
            &serde_json::json!({}),
        )]));

        // Act
        let topics = c.fetch_topics("owner/repo").unwrap();

        // Assert
        assert!(topics.is_empty());
    }

    // ------------------------------------------------------------------
    // fetch_labels
    // ------------------------------------------------------------------

    #[test]
    fn fetch_labels_calls_label_list_subcommand() {
        // Arrange
        let json = serde_json::json!([
            {"name": "bug", "color": "d73a4a", "description": "Something broken"},
            {"name": "enhancement", "color": "a2eeef", "description": ""}
        ]);
        let c = client(MockGhRunner::ok(serde_json::to_vec(&json).unwrap()));

        // Act
        let labels = c.fetch_labels("owner/repo").unwrap();

        // Assert
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0].name, "bug");
        assert_eq!(labels[0].color, "d73a4a");
        assert_eq!(labels[0].description, Some("Something broken".to_owned()));
        // Empty description becomes None
        assert_eq!(labels[1].description, None);
        let args = &c.runner.calls()[0];
        assert_eq!(args[0], "label");
        assert_eq!(args[1], "list");
    }

    #[test]
    fn fetch_labels_propagates_error() {
        // Arrange
        let c = client(MockGhRunner::err(b"gh: error".to_vec()));

        // Act
        let err = c.fetch_labels("owner/repo").unwrap_err();

        // Assert
        assert!(err.to_string().contains("`gh label list` failed"));
    }

    // ------------------------------------------------------------------
    // fetch_actions_permissions
    // ------------------------------------------------------------------

    #[test]
    fn fetch_actions_permissions_parses_response() {
        // Arrange
        let json = serde_json::json!({
            "enabled": true, "allowed_actions": "selected", "sha_pinning_required": false
        });
        let c = client(MockGhRunner::new(vec![success_json(&json)]));

        // Act
        let perm = c.fetch_actions_permissions("owner/repo").unwrap();

        // Assert
        assert!(perm.enabled);
        assert_eq!(perm.allowed_actions.as_deref(), Some("selected"));
        assert_eq!(perm.sha_pinning_required, Some(false));
    }

    #[test]
    fn fetch_actions_permissions_defaults_enabled_true_when_missing() {
        // Arrange — response without "enabled" key
        let c = client(MockGhRunner::new(vec![success_json(
            &serde_json::json!({}),
        )]));

        // Act
        let perm = c.fetch_actions_permissions("owner/repo").unwrap();

        // Assert
        assert!(perm.enabled, "missing 'enabled' should default to true");
    }

    // ------------------------------------------------------------------
    // fetch_selected_actions
    // ------------------------------------------------------------------

    #[test]
    fn fetch_selected_actions_parses_response() {
        // Arrange
        let json = serde_json::json!({
            "github_owned_allowed": true,
            "patterns_allowed": ["owner/action@v1"]
        });
        let c = client(MockGhRunner::new(vec![success_json(&json)]));

        // Act
        let sa = c.fetch_selected_actions("owner/repo").unwrap();

        // Assert
        assert_eq!(sa.github_owned_allowed, Some(true));
        assert_eq!(
            sa.patterns_allowed,
            Some(vec!["owner/action@v1".to_owned()])
        );
        assert!(
            c.runner.calls()[0]
                .contains(&"repos/owner/repo/actions/permissions/selected-actions".to_owned())
        );
    }

    // ------------------------------------------------------------------
    // fetch_workflow_permissions
    // ------------------------------------------------------------------

    #[test]
    fn fetch_workflow_permissions_parses_response() {
        // Arrange
        let json = serde_json::json!({
            "default_workflow_permissions": "read",
            "can_approve_pull_request_reviews": true
        });
        let c = client(MockGhRunner::new(vec![success_json(&json)]));

        // Act
        let wp = c.fetch_workflow_permissions("owner/repo").unwrap();

        // Assert
        assert_eq!(wp.default_workflow_permissions.as_deref(), Some("read"));
        assert!(wp.can_approve_pull_request_reviews);
    }

    // ------------------------------------------------------------------
    // fetch_ruleset_details
    // ------------------------------------------------------------------

    #[test]
    fn fetch_ruleset_details_calls_correct_endpoint() {
        // Arrange
        let json = serde_json::json!({"id": 42, "name": "protect-main"});
        let c = client(MockGhRunner::new(vec![success_json(&json)]));

        // Act
        let v = c.fetch_ruleset_details("owner/repo", 42).unwrap();

        // Assert
        assert_eq!(v["id"], 42);
        assert!(c.runner.calls()[0].contains(&"repos/owner/repo/rulesets/42".to_owned()));
    }

    // ------------------------------------------------------------------
    // fetch_rulesets
    // ------------------------------------------------------------------

    #[test]
    fn fetch_rulesets_parses_array() {
        // Arrange
        let json = serde_json::json!([
            {"id": 1, "name": "protect-main"},
            {"id": 2, "name": "protect-release"}
        ]);
        let c = client(MockGhRunner::new(vec![success_json(&json)]));

        // Act
        let rulesets = c.fetch_rulesets("owner/repo").unwrap();

        // Assert
        assert_eq!(rulesets.len(), 2);
        assert_eq!(rulesets[0].id, 1);
        assert_eq!(rulesets[0].name, "protect-main");
    }

    #[test]
    fn fetch_rulesets_error_on_non_array() {
        // Arrange
        let c = client(MockGhRunner::new(vec![success_json(
            &serde_json::json!({"error": "bad"}),
        )]));

        // Act
        let err = c.fetch_rulesets("owner/repo").unwrap_err();

        // Assert
        assert!(err.to_string().contains("not an array"));
    }

    // ------------------------------------------------------------------
    // fetch_branch_protection
    // ------------------------------------------------------------------

    #[test]
    fn fetch_branch_protection_returns_some_on_success() {
        // Arrange — minimal protection object
        let json = serde_json::json!({
            "required_status_checks": null,
            "enforce_admins": {"enabled": false},
            "required_pull_request_reviews": null,
            "restrictions": null,
            "required_linear_history": {"enabled": false},
            "allow_force_pushes": {"enabled": false},
            "allow_deletions": {"enabled": false},
            "required_conversation_resolution": {"enabled": false},
            "lock_branch": {"enabled": false},
            "allow_fork_syncing": {"enabled": false},
            "block_creations": {"enabled": false},
            "required_signatures": {"enabled": false}
        });
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: serde_json::to_vec(&json).unwrap(),
            stderr: vec![],
        }]));

        // Act
        let result = c.fetch_branch_protection("owner/repo", "main").unwrap();

        // Assert
        assert!(result.is_some());
    }

    #[test]
    fn fetch_branch_protection_returns_none_on_404_in_stderr() {
        // Arrange — exit code 1, "404" in stderr
        let c = client(MockGhRunner::new(vec![failure(
            1,
            "HTTP 404: Branch not protected",
        )]));

        // Act
        let result = c.fetch_branch_protection("owner/repo", "main").unwrap();

        // Assert
        assert!(result.is_none());
    }

    #[test]
    fn fetch_branch_protection_returns_none_on_branch_not_protected_in_stderr() {
        // Arrange
        let c = client(MockGhRunner::new(vec![failure(1, BRANCH_NOT_PROTECTED)]));

        // Act
        let result = c.fetch_branch_protection("owner/repo", "main").unwrap();

        // Assert
        assert!(result.is_none());
    }

    #[test]
    fn fetch_branch_protection_returns_none_on_message_in_json_body() {
        // Arrange — status success but body says not protected
        let json = serde_json::json!({"message": BRANCH_NOT_PROTECTED});
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: serde_json::to_vec(&json).unwrap(),
            stderr: vec![],
        }]));

        // Act
        let result = c.fetch_branch_protection("owner/repo", "main").unwrap();

        // Assert
        assert!(result.is_none());
    }

    #[test]
    fn fetch_branch_protection_error_on_unexpected_failure() {
        // Arrange — non-404/non-1 exit
        let c = client(MockGhRunner::new(vec![failure(2, "internal server error")]));

        // Act
        let err = c.fetch_branch_protection("owner/repo", "main").unwrap_err();

        // Assert
        assert!(err.to_string().contains("branch protection failed"));
    }

    // ------------------------------------------------------------------
    // list_protected_branches
    // ------------------------------------------------------------------

    #[test]
    fn list_protected_branches_returns_names() {
        // Arrange
        let json = serde_json::json!([
            {"name": "main", "protected": true},
            {"name": "release", "protected": true}
        ]);
        let c = client(MockGhRunner::new(vec![success_json(&json)]));

        // Act
        let branches = c.list_protected_branches("owner/repo").unwrap();

        // Assert
        assert_eq!(branches, ["main", "release"]);
        assert!(
            c.runner.calls()[0]
                .contains(&"repos/owner/repo/branches?protected=true&per_page=100".to_owned())
        );
    }

    // ------------------------------------------------------------------
    // fetch_release_immutability
    // ------------------------------------------------------------------

    #[test]
    fn fetch_release_immutability_returns_enabled() {
        // Arrange
        let json = serde_json::json!({"enabled": true});
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: serde_json::to_vec(&json).unwrap(),
            stderr: vec![],
        }]));

        // Act
        let result = c.fetch_release_immutability("owner/repo").unwrap();

        // Assert
        assert_eq!(result, Some(true));
    }

    #[test]
    fn fetch_release_immutability_returns_none_on_404() {
        // Arrange
        let c = client(MockGhRunner::new(vec![failure(1, "HTTP 404")]));

        // Act
        let result = c.fetch_release_immutability("owner/repo").unwrap();

        // Assert
        assert!(result.is_none());
    }

    #[test]
    fn fetch_release_immutability_returns_none_on_403() {
        // Arrange
        let c = client(MockGhRunner::new(vec![failure(1, "HTTP 403 Forbidden")]));

        // Act
        let result = c.fetch_release_immutability("owner/repo").unwrap();

        // Assert
        assert!(result.is_none());
    }

    #[test]
    fn fetch_release_immutability_error_on_other_failure() {
        // Arrange
        let c = client(MockGhRunner::new(vec![failure(1, "HTTP 500")]));

        // Act
        let err = c.fetch_release_immutability("owner/repo").unwrap_err();

        // Assert
        assert!(err.to_string().contains("immutable-releases"));
    }

    // ------------------------------------------------------------------
    // put_release_immutability
    // ------------------------------------------------------------------

    #[test]
    fn put_release_immutability_uses_put_when_enabled() {
        // Arrange
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: vec![],
            stderr: vec![],
        }]));

        // Act
        c.put_release_immutability("owner/repo", true).unwrap();

        // Assert
        let args = &c.runner.calls()[0];
        assert_eq!(args[0], "api");
        assert_eq!(args[1], "-X");
        assert_eq!(args[2], "PUT");
    }

    #[test]
    fn put_release_immutability_uses_delete_when_disabled() {
        // Arrange
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: vec![],
            stderr: vec![],
        }]));

        // Act
        c.put_release_immutability("owner/repo", false).unwrap();

        // Assert
        let args = &c.runner.calls()[0];
        assert_eq!(args[2], "DELETE");
    }

    #[test]
    fn put_release_immutability_error_on_failure() {
        // Arrange
        let c = client(MockGhRunner::err(b"forbidden".to_vec()));

        // Act
        let err = c.put_release_immutability("owner/repo", true).unwrap_err();

        // Assert
        assert!(err.to_string().contains("immutable-releases"));
    }

    // ------------------------------------------------------------------
    // fetch_fork_pr_approval
    // ------------------------------------------------------------------

    #[test]
    fn fetch_fork_pr_approval_returns_policy() {
        // Arrange
        let json = serde_json::json!({"approval_policy": "approved"});
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: serde_json::to_vec(&json).unwrap(),
            stderr: vec![],
        }]));

        // Act
        let result = c.fetch_fork_pr_approval("owner/repo").unwrap();

        // Assert
        assert_eq!(result, Some("approved".to_owned()));
    }

    #[test]
    fn fetch_fork_pr_approval_returns_none_on_404() {
        // Arrange
        let c = client(MockGhRunner::new(vec![failure(1, "HTTP 404")]));

        // Act
        let result = c.fetch_fork_pr_approval("owner/repo").unwrap();

        // Assert
        assert!(result.is_none());
    }

    #[test]
    fn fetch_fork_pr_approval_returns_none_on_422() {
        // Arrange
        let c = client(MockGhRunner::new(vec![failure(
            1,
            "HTTP 422 Unprocessable Entity",
        )]));

        // Act
        let result = c.fetch_fork_pr_approval("owner/repo").unwrap();

        // Assert
        assert!(result.is_none());
    }

    #[test]
    fn fetch_fork_pr_approval_error_on_other_failure() {
        // Arrange
        let c = client(MockGhRunner::new(vec![failure(1, "HTTP 500")]));

        // Act
        let err = c.fetch_fork_pr_approval("owner/repo").unwrap_err();

        // Assert
        assert!(err.to_string().contains("fork-pr-contributor-approval"));
    }

    // ------------------------------------------------------------------
    // put_fork_pr_approval
    // ------------------------------------------------------------------

    #[test]
    fn put_fork_pr_approval_sends_correct_body() {
        // Arrange
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: vec![],
            stderr: vec![],
        }]));

        // Act
        c.put_fork_pr_approval("owner/repo", "approved").unwrap();

        // Assert
        let stdin = c.runner.stdin_of(0).unwrap();
        let body: serde_json::Value = serde_json::from_slice(&stdin).unwrap();
        assert_eq!(body["approval_policy"], "approved");
        assert!(c.runner.calls()[0].contains(
            &"repos/owner/repo/actions/permissions/fork-pr-contributor-approval".to_owned()
        ));
    }

    // ------------------------------------------------------------------
    // patch_repo
    // ------------------------------------------------------------------

    #[test]
    fn patch_repo_sends_patch_request() {
        // Arrange
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: vec![],
            stderr: vec![],
        }]));
        let body = serde_json::json!({"description": "new desc"});

        // Act
        c.patch_repo("owner/repo", &body).unwrap();

        // Assert
        let args = &c.runner.calls()[0];
        assert_eq!(args[1], "-X");
        assert_eq!(args[2], "PATCH");
        assert_eq!(args[3], "repos/owner/repo");
    }

    // ------------------------------------------------------------------
    // put_topics
    // ------------------------------------------------------------------

    #[test]
    fn put_topics_sends_names_body() {
        // Arrange
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: vec![],
            stderr: vec![],
        }]));

        // Act
        c.put_topics("owner/repo", &["rust".to_owned(), "cli".to_owned()])
            .unwrap();

        // Assert
        let stdin = c.runner.stdin_of(0).unwrap();
        let body: serde_json::Value = serde_json::from_slice(&stdin).unwrap();
        assert_eq!(body["names"], serde_json::json!(["rust", "cli"]));
        assert_eq!(c.runner.calls()[0][2], "PUT");
        assert!(c.runner.calls()[0].contains(&"repos/owner/repo/topics".to_owned()));
    }

    // ------------------------------------------------------------------
    // create_label / update_label / delete_label
    // ------------------------------------------------------------------

    #[test]
    fn create_label_with_description_passes_flag() {
        // Arrange
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: vec![],
            stderr: vec![],
        }]));

        // Act
        c.create_label("owner/repo", "bug", "d73a4a", Some("Something broken"))
            .unwrap();

        // Assert
        let args = &c.runner.calls()[0];
        assert_eq!(args[0], "label");
        assert_eq!(args[1], "create");
        assert!(args.contains(&"--description".to_owned()));
        assert!(args.contains(&"Something broken".to_owned()));
    }

    #[test]
    fn create_label_without_description_omits_flag() {
        // Arrange
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: vec![],
            stderr: vec![],
        }]));

        // Act
        c.create_label("owner/repo", "bug", "d73a4a", None).unwrap();

        // Assert
        let args = &c.runner.calls()[0];
        assert!(!args.contains(&"--description".to_owned()));
    }

    #[test]
    fn create_label_error_on_failure() {
        // Arrange
        let c = client(MockGhRunner::err(b"already exists".to_vec()));

        // Act
        let err = c
            .create_label("owner/repo", "bug", "d73a4a", None)
            .unwrap_err();

        // Assert
        assert!(err.to_string().contains("`gh label create` failed"));
    }

    #[test]
    fn update_label_calls_edit_subcommand() {
        // Arrange
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: vec![],
            stderr: vec![],
        }]));

        // Act
        c.update_label("owner/repo", "bug", "ff0000", Some("Updated"))
            .unwrap();

        // Assert
        let args = &c.runner.calls()[0];
        assert_eq!(args[1], "edit");
        assert!(args.contains(&"--description".to_owned()));
    }

    #[test]
    fn delete_label_calls_delete_subcommand() {
        // Arrange
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: vec![],
            stderr: vec![],
        }]));

        // Act
        c.delete_label("owner/repo", "bug").unwrap();

        // Assert
        let args = &c.runner.calls()[0];
        assert_eq!(args[0], "label");
        assert_eq!(args[1], "delete");
        assert!(args.contains(&"--confirm".to_owned()));
    }

    // ------------------------------------------------------------------
    // put_actions_permissions / put_selected_actions / put_workflow_permissions
    // ------------------------------------------------------------------

    #[test]
    fn put_actions_permissions_uses_put_method() {
        // Arrange
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: vec![],
            stderr: vec![],
        }]));

        // Act
        c.put_actions_permissions("owner/repo", &serde_json::json!({"enabled": true}))
            .unwrap();

        // Assert
        assert_eq!(c.runner.calls()[0][2], "PUT");
        assert!(c.runner.calls()[0].contains(&"repos/owner/repo/actions/permissions".to_owned()));
    }

    #[test]
    fn put_selected_actions_uses_correct_endpoint() {
        // Arrange
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: vec![],
            stderr: vec![],
        }]));

        // Act
        c.put_selected_actions("owner/repo", &serde_json::json!({}))
            .unwrap();

        // Assert
        assert!(
            c.runner.calls()[0]
                .contains(&"repos/owner/repo/actions/permissions/selected-actions".to_owned())
        );
    }

    #[test]
    fn put_workflow_permissions_uses_correct_endpoint() {
        // Arrange
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: vec![],
            stderr: vec![],
        }]));

        // Act
        c.put_workflow_permissions("owner/repo", &serde_json::json!({}))
            .unwrap();

        // Assert
        assert!(
            c.runner.calls()[0]
                .contains(&"repos/owner/repo/actions/permissions/workflow".to_owned())
        );
    }

    // ------------------------------------------------------------------
    // create_ruleset / update_ruleset / delete_ruleset
    // ------------------------------------------------------------------

    #[test]
    fn create_ruleset_sends_post_request() {
        // Arrange
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: vec![],
            stderr: vec![],
        }]));

        // Act
        c.create_ruleset("owner/repo", &serde_json::json!({"name": "r"}))
            .unwrap();

        // Assert
        assert_eq!(c.runner.calls()[0][2], "POST");
        assert!(c.runner.calls()[0].contains(&"repos/owner/repo/rulesets".to_owned()));
    }

    #[test]
    fn update_ruleset_sends_put_with_id() {
        // Arrange
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: vec![],
            stderr: vec![],
        }]));

        // Act
        c.update_ruleset("owner/repo", 99, &serde_json::json!({}))
            .unwrap();

        // Assert
        assert_eq!(c.runner.calls()[0][2], "PUT");
        assert!(c.runner.calls()[0].contains(&"repos/owner/repo/rulesets/99".to_owned()));
    }

    #[test]
    fn delete_ruleset_sends_delete_method() {
        // Arrange
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: vec![],
            stderr: vec![],
        }]));

        // Act
        c.delete_ruleset("owner/repo", 5).unwrap();

        // Assert
        let args = &c.runner.calls()[0];
        assert_eq!(args[1], "-X");
        assert_eq!(args[2], "DELETE");
        assert!(args.contains(&"repos/owner/repo/rulesets/5".to_owned()));
    }

    #[test]
    fn delete_ruleset_error_on_failure() {
        // Arrange
        let c = client(MockGhRunner::err(b"not found".to_vec()));

        // Act
        let err = c.delete_ruleset("owner/repo", 1).unwrap_err();

        // Assert
        assert!(err.to_string().contains("rulesets/1"));
    }

    // ------------------------------------------------------------------
    // put_branch_protection / delete_branch_protection
    // ------------------------------------------------------------------

    #[test]
    fn put_branch_protection_uses_correct_endpoint() {
        // Arrange
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: vec![],
            stderr: vec![],
        }]));

        // Act
        c.put_branch_protection("owner/repo", "main", &serde_json::json!({}))
            .unwrap();

        // Assert
        assert!(
            c.runner.calls()[0].contains(&"repos/owner/repo/branches/main/protection".to_owned())
        );
        assert_eq!(c.runner.calls()[0][2], "PUT");
    }

    #[test]
    fn delete_branch_protection_sends_delete() {
        // Arrange
        let c = client(MockGhRunner::new(vec![GhOutput {
            exit_code: Some(0),
            stdout: vec![],
            stderr: vec![],
        }]));

        // Act
        c.delete_branch_protection("owner/repo", "main").unwrap();

        // Assert
        let args = &c.runner.calls()[0];
        assert_eq!(args[2], "DELETE");
        assert!(args.contains(&"repos/owner/repo/branches/main/protection".to_owned()));
    }

    #[test]
    fn delete_branch_protection_error_on_failure() {
        // Arrange
        let c = client(MockGhRunner::err(b"not found".to_vec()));

        // Act
        let err = c
            .delete_branch_protection("owner/repo", "main")
            .unwrap_err();

        // Assert
        assert!(err.to_string().contains("DELETE branch protection"));
    }

    // ------------------------------------------------------------------
    // resolve_team_id / resolve_app_id / resolve_org_custom_role_id
    // ------------------------------------------------------------------

    #[test]
    fn resolve_team_id_returns_id_field() {
        // Arrange
        let json = serde_json::json!({"id": 42, "slug": "engineers"});
        let c = client(MockGhRunner::new(vec![success_json(&json)]));

        // Act
        let id = c.resolve_team_id("my-org", "engineers").unwrap();

        // Assert
        assert_eq!(id, 42);
        assert!(c.runner.calls()[0].contains(&"orgs/my-org/teams/engineers".to_owned()));
    }

    #[test]
    fn resolve_team_id_error_on_missing_id() {
        // Arrange
        let c = client(MockGhRunner::new(vec![success_json(
            &serde_json::json!({"slug": "x"}),
        )]));

        // Act
        let err = c.resolve_team_id("my-org", "x").unwrap_err();

        // Assert
        assert!(err.to_string().contains("has no id field"));
    }

    #[test]
    fn resolve_app_id_returns_id_field() {
        // Arrange
        let json = serde_json::json!({"id": 99, "slug": "my-app"});
        let c = client(MockGhRunner::new(vec![success_json(&json)]));

        // Act
        let id = c.resolve_app_id("my-app").unwrap();

        // Assert
        assert_eq!(id, 99);
        assert!(c.runner.calls()[0].contains(&"apps/my-app".to_owned()));
    }

    #[test]
    fn resolve_org_custom_role_id_finds_role_by_name() {
        // Arrange — v1 format: top-level array
        let json = serde_json::json!([
            {"id": 10, "name": "reader"},
            {"id": 11, "name": "writer"}
        ]);
        let c = client(MockGhRunner::new(vec![success_json(&json)]));

        // Act
        let id = c.resolve_org_custom_role_id("my-org", "writer").unwrap();

        // Assert
        assert_eq!(id, 11);
    }

    #[test]
    fn resolve_org_custom_role_id_nested_custom_roles_key() {
        // Arrange — v2 format: {"custom_roles": [...]}
        let json = serde_json::json!({
            "custom_roles": [
                {"id": 7, "name": "auditor"}
            ]
        });
        let c = client(MockGhRunner::new(vec![success_json(&json)]));

        // Act
        let id = c.resolve_org_custom_role_id("my-org", "auditor").unwrap();

        // Assert
        assert_eq!(id, 7);
    }

    #[test]
    fn resolve_org_custom_role_id_error_when_not_found() {
        // Arrange
        let json = serde_json::json!([{"id": 1, "name": "other"}]);
        let c = client(MockGhRunner::new(vec![success_json(&json)]));

        // Act
        let err = c
            .resolve_org_custom_role_id("my-org", "missing")
            .unwrap_err();

        // Assert
        assert!(err.to_string().contains("not found in org"));
    }

    // ------------------------------------------------------------------
    // gh_api_write error path
    // ------------------------------------------------------------------

    #[test]
    fn gh_api_write_error_on_failure() {
        // Arrange — gh_api_write is tested indirectly through patch_repo
        let c = client(MockGhRunner::err(b"500 error".to_vec()));

        // Act
        let err = c
            .patch_repo("owner/repo", &serde_json::json!({"description": "x"}))
            .unwrap_err();

        // Assert
        assert!(
            err.to_string()
                .contains("`gh api PATCH repos/owner/repo` failed")
        );
    }

    // ------------------------------------------------------------------
    // execute_inner (existing tests — behavior unchanged)
    // ------------------------------------------------------------------

    #[test]
    fn execute_inner_invalid_manifest_returns_failure_before_api() {
        // An invalid manifest (pattern without '@') must abort before any API call.
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(
            &path,
            "upstream:\n  repo: owner/repo\n\
             spec:\n  actions:\n    enabled: true\n    allowed_actions: selected\n\
             \n    selected_actions:\n      patterns_allowed:\n        - 'jdx/mise-action'\n\
             files:\n  - path: foo.txt\n    strategy: replace\n",
        )
        .unwrap();
        let args = super::super::cli::SyncRepoArgs {
            manifest: path,
            dry_run: false,
            ci_check: false,
            yes: false,
        };
        let client_mock = MockRepoClient::new("owner/repo");
        let mut buf: Vec<u8> = Vec::new();
        let code = execute_inner(&args, &client_mock, &mut buf);
        // Must fail due to validation, before any API calls.
        assert_eq!(code, ExitCode::FAILURE);
        assert!(
            client_mock.detect_repo_calls.get() == 0,
            "API must not be called when validation fails"
        );
    }

    #[test]
    fn execute_inner_no_spec_returns_success() {
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(
            &path,
            "upstream:\n  repo: owner/repo\nfiles:\n  - path: foo.txt\n    strategy: replace\n",
        )
        .unwrap();
        let args = super::super::cli::SyncRepoArgs {
            manifest: path,
            dry_run: false,
            ci_check: false,
            yes: false,
        };
        let client_mock = MockRepoClient::new("owner/repo");
        let mut buf: Vec<u8> = Vec::new();
        let code = execute_inner(&args, &client_mock, &mut buf);
        assert_eq!(code, ExitCode::SUCCESS);
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("nothing to do"), "unexpected: {out}");
    }

    #[test]
    fn execute_inner_ci_check_exits_failure_on_drift() {
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(
            &path,
            "upstream:\n  repo: owner/repo\nspec:\n  visibility: private\nfiles:\n  - path: foo.txt\n    strategy: replace\n",
        )
        .unwrap();
        let args = super::super::cli::SyncRepoArgs {
            manifest: path,
            dry_run: false,
            ci_check: true,
            yes: false,
        };
        let client_mock = MockRepoClient::new("owner/repo");
        let mut buf: Vec<u8> = Vec::new();
        let code = execute_inner(&args, &client_mock, &mut buf);
        assert_eq!(code, ExitCode::FAILURE);
    }

    #[test]
    fn execute_inner_dry_run_no_apply() {
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(
            &path,
            "upstream:\n  repo: owner/repo\nspec:\n  description: new\nfiles:\n  - path: foo.txt\n    strategy: replace\n",
        )
        .unwrap();
        let args = super::super::cli::SyncRepoArgs {
            manifest: path,
            dry_run: true,
            ci_check: false,
            yes: false,
        };
        let client_mock = MockRepoClient::new("owner/repo");
        let mut buf: Vec<u8> = Vec::new();
        let code = execute_inner(&args, &client_mock, &mut buf);
        assert_eq!(code, ExitCode::SUCCESS);
        assert!(client_mock.applied_patches.borrow().is_empty());
    }
}
