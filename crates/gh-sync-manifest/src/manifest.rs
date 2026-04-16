use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{SyncError, ValidationError};

// ---------------------------------------------------------------------------
// Schema types
// ---------------------------------------------------------------------------

/// Top-level manifest structure.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Upstream template repository configuration.
    pub upstream: Upstream,
    /// Repository settings (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec: Option<Spec>,
    /// Synchronisation file rules (must be non-empty).
    pub files: Vec<Rule>,
}

/// Repository settings block (compatible with gh-infra `Kind: Repository` spec).
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    /// Short description of the repository.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Repository homepage URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// Repository visibility: `public`, `private`, or `internal`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    /// Whether the repository is archived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    /// Topic tags for the repository (full list, not additive).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topics: Option<Vec<String>>,
    /// Optional repository feature toggles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features: Option<Features>,
    /// Require contributors to sign off on web-based commits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_commit_signoff_required: Option<bool>,
    /// Merge strategy settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_strategy: Option<MergeStrategy>,
    /// Lock releases after publishing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_immutability: Option<bool>,
    /// Label sync mode: `additive` (default) or `mirror`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_sync: Option<String>,
    /// Managed labels list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<Label>>,
    /// GitHub Actions settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<Actions>,
    /// Repository rulesets (prefer over `branch_protection`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rulesets: Option<Vec<Ruleset>>,
    /// Classic branch protection rules.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_protection: Option<Vec<BranchProtection>>,
}

/// Repository feature toggles.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Features {
    /// Enable or disable the Issues tab.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issues: Option<bool>,
    /// Enable or disable the Projects tab.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projects: Option<bool>,
    /// Enable or disable the Wiki tab.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wiki: Option<bool>,
    /// Enable or disable the Discussions tab.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discussions: Option<bool>,
}

/// Merge strategy settings.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MergeStrategy {
    /// Allow merge commits (creates a merge commit).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_merge_commit: Option<bool>,
    /// Allow squash merging (combines all commits into one).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_squash_merge: Option<bool>,
    /// Allow rebase merging (replays commits onto the base branch).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_rebase_merge: Option<bool>,
    /// Allow auto-merge to automatically merge when all checks pass.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_auto_merge: Option<bool>,
    /// Allow contributors to update pull request branches.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_update_branch: Option<bool>,
    /// Automatically delete head branches after a pull request is merged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_delete_head_branches: Option<bool>,
    /// Title format for merge commits: `PR_TITLE` or `MERGE_MESSAGE`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_commit_title: Option<String>,
    /// Body format for merge commits: `PR_BODY`, `PR_TITLE`, or `BLANK`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_commit_message: Option<String>,
    /// Title format for squash commits: `PR_TITLE` or `COMMIT_OR_PR_TITLE`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub squash_merge_commit_title: Option<String>,
    /// Body format for squash commits: `PR_BODY`, `COMMIT_MESSAGES`, or `BLANK`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub squash_merge_commit_message: Option<String>,
}

/// A managed repository label.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Label {
    /// Label name (must be unique).
    pub name: String,
    /// Hex colour without `#`.
    pub color: String,
    /// Optional human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// GitHub Actions settings.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_field_names)]
pub struct Actions {
    /// Must be set when any other `actions.*` field is present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// `all`, `local_only`, or `selected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_actions: Option<String>,
    /// Require full SHA pinning for external actions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha_pinning_required: Option<bool>,
    /// Default token permissions: `read` or `write`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_permissions: Option<String>,
    /// Allow workflows to approve pull requests.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub can_approve_pull_requests: Option<bool>,
    /// Allowed action patterns (requires `allowed_actions: selected`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_actions: Option<SelectedActions>,
    /// Fork PR approval policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fork_pr_approval: Option<String>,
}

/// Allowed action patterns for `allowed_actions: selected`.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedActions {
    /// Allow actions created by GitHub (e.g. `actions/*`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_owned_allowed: Option<bool>,
    /// Glob patterns for externally-authored actions that are permitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patterns_allowed: Option<Vec<String>>,
}

/// A repository ruleset.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Ruleset {
    /// Unique ruleset name.
    pub name: String,
    /// `branch` or `tag`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// `active`, `disabled`, or `evaluate`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enforcement: Option<String>,
    /// Actors that may bypass this ruleset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bypass_actors: Option<Vec<BypassActor>>,
    /// Ref name conditions for this ruleset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditions: Option<RulesetConditions>,
    /// Rules to enforce.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules: Option<RulesetRules>,
}

/// A bypass actor for a ruleset.
///
/// Exactly one of `role`, `team`, `app`, `org_admin`, or `custom_role` must be set.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BypassActor {
    /// Built-in role name: `admin`, `maintain`, `write`, `triage`, or `read`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// GitHub team slug (without the `@org/` prefix).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    /// GitHub App slug that may bypass the ruleset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// Grant bypass to all organisation admins when `true`.
    #[serde(rename = "org-admin", skip_serializing_if = "Option::is_none")]
    pub org_admin: Option<bool>,
    /// Custom repository role name that may bypass the ruleset.
    #[serde(rename = "custom-role", skip_serializing_if = "Option::is_none")]
    pub custom_role: Option<String>,
    /// `always`, `pull_request`, or `exempt`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bypass_mode: Option<String>,
}

/// Conditions for a ruleset.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RulesetConditions {
    /// Ref name include/exclude filter applied to the ruleset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_name: Option<RefNameCondition>,
}

/// `fnmatch`-style ref name include/exclude patterns.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RefNameCondition {
    /// Patterns that must match for the ruleset to apply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
    /// Patterns that, when matched, exempt the ref from the ruleset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<String>>,
}

/// Rules within a ruleset.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RulesetRules {
    /// Block force pushes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub non_fast_forward: Option<bool>,
    /// Block ref deletion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deletion: Option<bool>,
    /// Block ref creation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation: Option<bool>,
    /// Require linear commit history.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_linear_history: Option<bool>,
    /// Require signed commits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_signatures: Option<bool>,
    /// Pull request review requirements.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<PullRequestRule>,
    /// Required status checks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_status_checks: Option<RequiredStatusChecks>,
}

/// Pull request review rule.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PullRequestRule {
    /// Minimum number of approving reviews required before merging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_approving_review_count: Option<u32>,
    /// Dismiss approved reviews when new commits are pushed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dismiss_stale_reviews_on_push: Option<bool>,
    /// Require review from a code owner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_code_owner_review: Option<bool>,
    /// Require approval of the most recent push.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_last_push_approval: Option<bool>,
    /// Require all review threads to be resolved before merging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_review_thread_resolution: Option<bool>,
    /// Allowed merge methods: `squash`, `merge`, `rebase`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_merge_methods: Option<Vec<String>>,
}

/// Required status checks rule.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredStatusChecks {
    /// Require branches to be up-to-date before merging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict_required_status_checks_policy: Option<bool>,
    /// Status check contexts that must pass before merging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contexts: Option<Vec<StatusCheckContext>>,
}

/// A single status check context.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StatusCheckContext {
    /// Check name as it appears in the GitHub UI (e.g. `"ci / test"`).
    pub context: String,
    /// App slug resolved to an `integration_id` at apply-time via the GitHub
    /// Apps API.  When omitted, defaults to `"github-actions"`.
    /// Mutually exclusive with `integration_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// Pass an `integration_id` directly without API resolution.
    /// Takes precedence over `app` when both are set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_id: Option<u64>,
}

/// Classic branch protection rule.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BranchProtection {
    /// Branch name pattern (must be unique).
    pub pattern: String,
    /// Number of required approving reviews before merging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_reviews: Option<u32>,
    /// Dismiss pull request approvals when new commits are pushed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dismiss_stale_reviews: Option<bool>,
    /// Require review from a designated code owner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_code_owner_reviews: Option<bool>,
    /// Required status checks that must pass before merging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_status_checks: Option<BranchProtectionStatusChecks>,
    /// Enforce all configured restrictions for administrators.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enforce_admins: Option<bool>,
    /// Permit force pushes to this branch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_force_pushes: Option<bool>,
    /// Permit deletion of this branch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_deletions: Option<bool>,
}

/// Status check settings for classic branch protection.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BranchProtectionStatusChecks {
    /// Require branches to be up-to-date with the base branch before merging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
    /// Status check context names that must pass.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contexts: Option<Vec<String>>,
}

/// Upstream repository reference.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Upstream {
    /// `owner/name` format GitHub repository.
    pub repo: String,
    /// Git ref (branch, tag, or commit SHA). Defaults to `"main"`.
    #[serde(rename = "ref", default = "default_ref")]
    pub ref_: String,
}

fn default_ref() -> String {
    String::from("main")
}

/// A single synchronisation rule.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Destination path in the downstream repository (relative, no leading `/`).
    pub path: String,
    /// Synchronisation strategy.
    pub strategy: Strategy,
    /// Override for the upstream source path (`replace`/`create_only` only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Explicit patch file path (`patch` strategy only; defaults to convention).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
}

/// Synchronisation strategy for a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    /// Overwrite local file with upstream content unconditionally.
    Replace,
    /// Create the file only if it does not yet exist locally.
    CreateOnly,
    /// Delete the local file if it exists.
    Delete,
    /// Apply a unified diff patch on top of the upstream file.
    Patch,
    /// Exclude this path from sync entirely.
    ///
    /// Used in a local overlay manifest to cancel an upstream rule for the
    /// same `path`. The file is never downloaded, patched, deleted, or
    /// checked for drift.
    Ignore,
}

impl std::fmt::Display for Strategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Replace => "replace",
            Self::CreateOnly => "create_only",
            Self::Delete => "delete",
            Self::Patch => "patch",
            Self::Ignore => "ignore",
        })
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

impl Manifest {
    /// Load and parse a manifest from a YAML file.
    ///
    /// # Errors
    /// Returns [`SyncError::ManifestLoad`] when the file cannot be read or
    /// the YAML content does not match the expected schema.
    pub fn load(path: &Path) -> Result<Self, SyncError> {
        fn inner(path: &Path) -> anyhow::Result<Manifest> {
            use anyhow::Context as _;
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read '{}'", path.display()))?;
            serde_yml::from_str(&content)
                .with_context(|| format!("failed to parse '{}'", path.display()))
        }
        inner(path).map_err(|source| SyncError::ManifestLoad {
            path: path.to_owned(),
            source,
        })
    }
}

// ---------------------------------------------------------------------------
// Stage 1 — schema validation (offline, no network)
// ---------------------------------------------------------------------------

/// Validate the manifest schema without accessing the network or filesystem.
///
/// Checks:
/// 1. `upstream.repo` matches `owner/name` pattern
/// 2. `rules` is non-empty
/// 3. Path constraints on `path` and `source`
/// 4. No duplicate `path` values
/// 5. Field combination rules per strategy
///
/// # Errors
/// Returns [`SyncError::Validation`] if one or more constraints are violated.
#[allow(clippy::too_many_lines)]
pub fn validate_schema(manifest: &Manifest) -> Result<(), SyncError> {
    let mut errors: Vec<ValidationError> = Vec::new();

    // 1. upstream.repo pattern
    if !is_valid_repo(&manifest.upstream.repo) {
        errors.push(ValidationError::top_level(
            "upstream.repo",
            format!(
                "must match owner/name (alphanumeric, '.', '_', '-'); got '{}'",
                manifest.upstream.repo
            ),
        ));
    }

    // 2. files non-empty
    if manifest.files.is_empty() {
        errors.push(ValidationError::top_level("files", "must not be empty"));
    }

    // Per-rule checks
    let mut seen_paths: Vec<&str> = Vec::new();
    for (i, rule) in manifest.files.iter().enumerate() {
        // 3. path constraints
        if let Some(msg) = validate_path(&rule.path) {
            errors.push(ValidationError::rule(i, "path", msg));
        }

        // 4. duplicate path
        if seen_paths.contains(&rule.path.as_str()) {
            errors.push(ValidationError::rule(
                i,
                "path",
                format!("duplicate path '{}'", rule.path),
            ));
        } else {
            seen_paths.push(&rule.path);
        }

        // 3. source constraints (when present)
        if let Some(src) = &rule.source
            && let Some(msg) = validate_path(src)
        {
            errors.push(ValidationError::rule(i, "source", msg));
        }

        // 5. field combination rules
        match rule.strategy {
            Strategy::Delete => {
                if rule.source.is_some() {
                    errors.push(ValidationError::rule(
                        i,
                        "source",
                        "field not allowed for strategy 'delete'",
                    ));
                }
                if rule.patch.is_some() {
                    errors.push(ValidationError::rule(
                        i,
                        "patch",
                        "field not allowed for strategy 'delete'",
                    ));
                }
            }
            Strategy::Patch => {
                if rule.source.is_some() {
                    errors.push(ValidationError::rule(
                        i,
                        "source",
                        "field not allowed for strategy 'patch' (source and path must be identical)",
                    ));
                }
            }
            Strategy::Replace => {
                if rule.patch.is_some() {
                    errors.push(ValidationError::rule(
                        i,
                        "patch",
                        "field not allowed for strategy 'replace'",
                    ));
                }
            }
            Strategy::CreateOnly => {
                if rule.patch.is_some() {
                    errors.push(ValidationError::rule(
                        i,
                        "patch",
                        "field not allowed for strategy 'create_only'",
                    ));
                }
            }
            Strategy::Ignore => {
                if rule.source.is_some() {
                    errors.push(ValidationError::rule(
                        i,
                        "source",
                        "field not allowed for strategy 'ignore'",
                    ));
                }
                if rule.patch.is_some() {
                    errors.push(ValidationError::rule(
                        i,
                        "patch",
                        "field not allowed for strategy 'ignore'",
                    ));
                }
            }
        }
    }

    // spec.actions.selected_actions.patterns_allowed checks
    if let Some(spec) = &manifest.spec
        && let Some(actions) = &spec.actions
        && let Some(sel) = &actions.selected_actions
        && let Some(patterns) = &sel.patterns_allowed
    {
        validate_action_patterns(patterns, &mut errors);
    }

    // spec.merge_strategy: at least one merge method must be enabled
    if let Some(spec) = &manifest.spec
        && let Some(ms) = &spec.merge_strategy
        && ms.allow_merge_commit == Some(false)
        && ms.allow_squash_merge == Some(false)
        && ms.allow_rebase_merge == Some(false)
    {
        errors.push(ValidationError::top_level(
            "spec.merge_strategy",
            "at least one merge method must be enabled (allow_merge_commit, allow_squash_merge, or allow_rebase_merge)",
        ));
    }

    if let Some(spec) = &manifest.spec
        && let Some(actions) = &spec.actions
        && let Some(value) = &actions.allowed_actions
        && !matches!(value.as_str(), "all" | "local_only" | "selected")
    {
        errors.push(ValidationError::top_level(
            "spec.actions.allowed_actions",
            format!("must be one of 'all', 'local_only', 'selected'; got '{value}'"),
        ));
    }

    // spec.actions.selected_actions: required when allowed_actions is "selected"
    if let Some(spec) = &manifest.spec
        && let Some(actions) = &spec.actions
        && actions.allowed_actions.as_deref() == Some("selected")
        && (actions.selected_actions.is_none()
            || actions.selected_actions.as_ref().is_some_and(|sa| {
                sa.github_owned_allowed.is_none() && sa.patterns_allowed.is_none()
            }))
    {
        errors.push(ValidationError::top_level(
            "spec.actions.selected_actions",
            "required when allowed_actions is 'selected': set github_owned_allowed or patterns_allowed",
        ));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(SyncError::Validation(errors))
    }
}

/// Validate each entry in `patterns_allowed`.
///
/// Two rules are enforced:
/// - The owner portion must be all-lowercase: GitHub normalises action refs to
///   lowercase before pattern matching, so mixed-case owners never match.
/// - Every pattern must contain `@`: without a ref specifier the pattern only
///   matches the bare action name and will not match SHA-pinned or tagged refs.
fn validate_action_patterns(patterns: &[String], errors: &mut Vec<ValidationError>) {
    for (i, pat) in patterns.iter().enumerate() {
        let field = format!("spec.actions.selected_actions.patterns_allowed[{i}]");
        let owner = pat.split('/').next().unwrap_or(pat);
        if owner.chars().any(char::is_uppercase) {
            errors.push(ValidationError::top_level(
                field.clone(),
                format!(
                    "owner '{owner}' contains uppercase letters; \
                     GitHub normalises action refs to lowercase so this \
                     pattern will never match — use all-lowercase (e.g. '{}')",
                    pat.to_lowercase()
                ),
            ));
        }
        if !pat.contains('@') {
            errors.push(ValidationError::top_level(
                field,
                format!(
                    "pattern '{pat}' has no ref specifier; \
                     it will not match SHA-pinned or tagged actions — \
                     use '{pat}@*' to allow any ref"
                ),
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Stage 2 — reference validation (local filesystem)
// ---------------------------------------------------------------------------

/// Validate that patch files referenced by `patch` rules exist on disk.
///
/// The `repo_root` is the directory from which relative patch paths are resolved.
///
/// # Errors
/// Returns [`SyncError::Validation`] if any patch file is missing.
pub fn validate_references(manifest: &Manifest, repo_root: &Path) -> Result<(), SyncError> {
    let mut errors: Vec<ValidationError> = Vec::new();

    for (i, rule) in manifest.files.iter().enumerate() {
        if rule.strategy != Strategy::Patch {
            continue;
        }

        let patch_path = resolve_patch_path(rule);
        let full_path = repo_root.join(&patch_path);
        if !full_path.exists() {
            errors.push(ValidationError::rule(
                i,
                "patch",
                format!("patch file not found: {patch_path}"),
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(SyncError::Validation(errors))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return the resolved patch file path for a `patch` strategy rule.
///
/// Uses `rule.patch` when specified, otherwise falls back to the convention:
/// `.github/gh-sync/patches/<path>.patch`.
#[must_use]
pub fn resolve_patch_path(rule: &Rule) -> String {
    rule.patch
        .clone()
        .unwrap_or_else(|| format!(".github/gh-sync/patches/{}.patch", rule.path))
}

/// Return `true` when `repo` matches the `owner/name` pattern.
///
/// Both segments allow ASCII alphanumerics plus `.`, `_`, and `-`.
/// Neither segment may be empty.
fn is_valid_repo(repo: &str) -> bool {
    let Some((owner, name)) = repo.split_once('/') else {
        return false;
    };
    // Reject a second slash (three-part path)
    if name.contains('/') {
        return false;
    }
    is_valid_repo_segment(owner) && is_valid_repo_segment(name)
}

fn is_valid_repo_segment(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

/// Return an error message if `path` violates the path constraints, or `None`
/// if the path is valid.
fn validate_path(path: &str) -> Option<String> {
    if path.is_empty() {
        return Some(String::from("must not be empty"));
    }
    if path.starts_with('/') {
        return Some(format!("must be relative (no leading '/'): '{path}'"));
    }
    if path.starts_with("./") {
        return Some(format!("must not start with './' : '{path}'"));
    }
    if path.ends_with('/') {
        return Some(format!("must not end with '/' : '{path}'"));
    }
    if path.contains("//") {
        return Some(format!("must not contain '//' : '{path}'"));
    }
    if path.contains('\\') {
        return Some(format!("must not contain '\\' (use '/' instead): '{path}'"));
    }
    // Check every segment for ".."
    if path.split('/').any(|seg| seg == "..") {
        return Some(format!("must not contain '..' segments: '{path}'"));
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::panic)] // test helpers use panic! for assertion failures
#[allow(clippy::unwrap_used)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // --- helpers ---

    fn manifest_from_yaml(yaml: &str) -> Manifest {
        serde_yml::from_str(yaml).expect("test YAML should be valid")
    }

    fn expect_schema_error(yaml: &str, field: &'static str) {
        let m = manifest_from_yaml(yaml);
        let Err(SyncError::Validation(errors)) = validate_schema(&m) else {
            panic!("expected validation error for field '{field}', got Ok");
        };
        assert!(
            errors.iter().any(|e| e.field == field),
            "expected error on field '{field}', got: {errors:?}"
        );
    }

    fn expect_schema_ok(yaml: &str) {
        let m = manifest_from_yaml(yaml);
        if let Err(e) = validate_schema(&m) {
            panic!("expected Ok, got: {e}");
        }
    }

    // --- is_valid_repo ---

    #[test]
    fn test_valid_repo_simple() {
        assert!(is_valid_repo("owner/name"));
    }

    #[test]
    fn test_valid_repo_with_special_chars() {
        assert!(is_valid_repo("my-org/my_repo.2"));
    }

    #[test]
    fn test_invalid_repo_no_slash() {
        assert!(!is_valid_repo("noowner"));
    }

    #[test]
    fn test_invalid_repo_empty_owner() {
        assert!(!is_valid_repo("/name"));
    }

    #[test]
    fn test_invalid_repo_empty_name() {
        assert!(!is_valid_repo("owner/"));
    }

    #[test]
    fn test_invalid_repo_three_parts() {
        assert!(!is_valid_repo("owner/repo/extra"));
    }

    #[test]
    fn test_invalid_repo_space() {
        assert!(!is_valid_repo("ow ner/name"));
    }

    // --- validate_path ---

    #[test]
    fn test_valid_path() {
        assert_eq!(validate_path(".github/workflows/ci.yaml"), None);
    }

    #[test]
    fn test_path_empty() {
        assert!(validate_path("").is_some());
    }

    #[test]
    fn test_path_absolute() {
        assert!(validate_path("/etc/passwd").is_some());
    }

    #[test]
    fn test_path_dotslash() {
        assert!(validate_path("./foo").is_some());
    }

    #[test]
    fn test_path_trailing_slash() {
        assert!(validate_path("foo/").is_some());
    }

    #[test]
    fn test_path_double_slash() {
        assert!(validate_path("foo//bar").is_some());
    }

    #[test]
    fn test_path_backslash() {
        assert!(validate_path("foo\\bar").is_some());
    }

    #[test]
    fn test_path_dotdot() {
        assert!(validate_path("../escape").is_some());
    }

    #[test]
    fn test_path_dotdot_middle() {
        assert!(validate_path("foo/../bar").is_some());
    }

    // --- validate_schema: upstream.repo ---

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_schema_invalid_repo_pattern() {
        expect_schema_error(
            r"
upstream:
  repo: 'invalid repo!'
files:
  - path: foo.txt
    strategy: replace
",
            "upstream.repo",
        );
    }

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_schema_valid_repo_pattern() {
        expect_schema_ok(
            r"
upstream:
  repo: 'owner/repo'
files:
  - path: foo.txt
    strategy: replace
",
        );
    }

    // --- validate_schema: files non-empty ---

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_schema_empty_rules() {
        expect_schema_error(
            r"
upstream:
  repo: owner/repo
files: []
",
            "files",
        );
    }

    // --- validate_schema: path constraints ---

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_schema_path_absolute() {
        expect_schema_error(
            r"
upstream:
  repo: owner/repo
files:
  - path: /absolute/path
    strategy: replace
",
            "path",
        );
    }

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_schema_path_dotdot() {
        expect_schema_error(
            r"
upstream:
  repo: owner/repo
files:
  - path: ../escape
    strategy: replace
",
            "path",
        );
    }

    // --- validate_schema: duplicate path ---

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_schema_duplicate_path() {
        expect_schema_error(
            r"
upstream:
  repo: owner/repo
files:
  - path: foo.txt
    strategy: replace
  - path: foo.txt
    strategy: create_only
",
            "path",
        );
    }

    // --- validate_schema: strategy field combinations ---

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_schema_delete_with_source() {
        expect_schema_error(
            r"
upstream:
  repo: owner/repo
files:
  - path: foo.txt
    strategy: delete
    source: bar.txt
",
            "source",
        );
    }

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_schema_delete_with_patch() {
        expect_schema_error(
            r"
upstream:
  repo: owner/repo
files:
  - path: foo.txt
    strategy: delete
    patch: foo.patch
",
            "patch",
        );
    }

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_schema_patch_with_source() {
        expect_schema_error(
            r"
upstream:
  repo: owner/repo
files:
  - path: foo.txt
    strategy: patch
    source: bar.txt
",
            "source",
        );
    }

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_schema_replace_with_patch_field() {
        expect_schema_error(
            r"
upstream:
  repo: owner/repo
files:
  - path: foo.txt
    strategy: replace
    patch: foo.patch
",
            "patch",
        );
    }

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_schema_create_only_with_patch_field() {
        expect_schema_error(
            r"
upstream:
  repo: owner/repo
files:
  - path: foo.txt
    strategy: create_only
    patch: foo.patch
",
            "patch",
        );
    }

    // --- validate_schema: valid full manifest ---

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_schema_valid_all_strategies() {
        expect_schema_ok(
            r"
upstream:
  repo: naa0yama/boilerplate-rust
  ref: main
spec:
  visibility: public
  archived: false
files:
  - path: .github/workflows/ci.yaml
    strategy: replace
  - path: .github/workflows/ci2.yaml
    strategy: replace
    source: templates/ci.yaml
  - path: bootstrap.sh
    strategy: create_only
  - path: legacy.sh
    strategy: delete
  - path: Cargo.toml
    strategy: patch
  - path: other.toml
    strategy: patch
    patch: custom/other.toml.patch
  - path: optional.txt
    strategy: ignore
",
        );
    }

    // --- validate_references ---

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_references_patch_file_exists() {
        let dir = TempDir::new().unwrap();
        let patch_dir = dir.path().join(".github/gh-sync/patches");
        std::fs::create_dir_all(&patch_dir).unwrap();
        std::fs::write(patch_dir.join("Cargo.toml.patch"), b"--- a\n+++ b\n").unwrap();

        let manifest = manifest_from_yaml(
            r"
upstream:
  repo: owner/repo
files:
  - path: Cargo.toml
    strategy: patch
",
        );
        assert!(validate_references(&manifest, dir.path()).is_ok());
    }

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_references_patch_file_missing() {
        let dir = TempDir::new().unwrap();

        let manifest = manifest_from_yaml(
            r"
upstream:
  repo: owner/repo
files:
  - path: Cargo.toml
    strategy: patch
",
        );
        let Err(SyncError::Validation(errors)) = validate_references(&manifest, dir.path()) else {
            panic!("expected validation error for missing patch file");
        };
        assert!(errors.iter().any(|e| e.field == "patch"));
    }

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_references_explicit_patch_path() {
        let dir = TempDir::new().unwrap();
        let custom_dir = dir.path().join("custom");
        std::fs::create_dir_all(&custom_dir).unwrap();
        std::fs::write(custom_dir.join("cargo.patch"), b"--- a\n+++ b\n").unwrap();

        let manifest = manifest_from_yaml(
            r"
upstream:
  repo: owner/repo
files:
  - path: Cargo.toml
    strategy: patch
    patch: custom/cargo.patch
",
        );
        assert!(validate_references(&manifest, dir.path()).is_ok());
    }

    // --- validate_schema: full spec with all new fields ---

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_schema_full_spec_deserialization() {
        let m = manifest_from_yaml(
            r"
upstream:
  repo: naa0yama/boilerplate-rust
  ref: main
spec:
  description: 'My project'
  homepage: 'https://example.com'
  visibility: public
  archived: false
  topics: [rust, cli]
  release_immutability: true
  label_sync: mirror
  labels:
    - name: kind/bug
      color: d73a4a
      description: 'A bug'
  features:
    issues: true
    projects: false
    wiki: false
    discussions: false
  merge_strategy:
    allow_merge_commit: false
    allow_squash_merge: true
    allow_rebase_merge: false
    auto_delete_head_branches: true
    merge_commit_title: MERGE_MESSAGE
    merge_commit_message: PR_TITLE
    squash_merge_commit_title: PR_TITLE
    squash_merge_commit_message: COMMIT_MESSAGES
  actions:
    enabled: true
    allowed_actions: selected
    sha_pinning_required: true
    workflow_permissions: read
    can_approve_pull_requests: false
    selected_actions:
      github_owned_allowed: true
      patterns_allowed:
        - 'actions/*'
    fork_pr_approval: all_external_contributors
  rulesets:
    - name: protect-main
      target: branch
      enforcement: active
      bypass_actors:
        - role: admin
          bypass_mode: always
        - org-admin: true
          bypass_mode: pull_request
        - custom-role: deployer
          bypass_mode: exempt
      conditions:
        ref_name:
          include: ['refs/heads/main']
          exclude: []
      rules:
        non_fast_forward: true
        deletion: true
        creation: false
        required_linear_history: true
        required_signatures: true
        pull_request:
          required_approving_review_count: 1
        required_status_checks:
          strict_required_status_checks_policy: true
          contexts:
            - context: 'ci/test'
              app: github-actions
  branch_protection:
    - pattern: main
      required_reviews: 1
      dismiss_stale_reviews: true
      require_code_owner_reviews: false
      require_status_checks:
        strict: true
        contexts: ['ci / test']
      enforce_admins: false
      allow_force_pushes: false
      allow_deletions: false
files:
  - path: Cargo.toml
    strategy: replace
",
        );
        let spec = m.spec.as_ref().unwrap();
        assert_eq!(spec.description.as_deref(), Some("My project"));
        assert_eq!(spec.visibility.as_deref(), Some("public"));
        let features = spec.features.as_ref().unwrap();
        assert_eq!(features.issues, Some(true));
        let ms = spec.merge_strategy.as_ref().unwrap();
        assert_eq!(ms.allow_squash_merge, Some(true));
        let actions = spec.actions.as_ref().unwrap();
        assert_eq!(actions.enabled, Some(true));
        let sa = actions.selected_actions.as_ref().unwrap();
        assert_eq!(sa.github_owned_allowed, Some(true));
        let rulesets = spec.rulesets.as_ref().unwrap();
        assert_eq!(rulesets.len(), 1);
        let rs = &rulesets[0];
        assert_eq!(rs.name, "protect-main");
        let actors = rs.bypass_actors.as_ref().unwrap();
        assert_eq!(actors.len(), 3);
        assert_eq!(actors[1].org_admin, Some(true));
        assert_eq!(actors[2].custom_role.as_deref(), Some("deployer"));
        let rules = rs.rules.as_ref().unwrap();
        assert_eq!(rules.non_fast_forward, Some(true));
        let pr_rule = rules.pull_request.as_ref().unwrap();
        assert_eq!(pr_rule.required_approving_review_count, Some(1));
        let bp = spec.branch_protection.as_ref().unwrap();
        assert_eq!(bp.len(), 1);
        assert_eq!(bp[0].pattern, "main");
        let labels = spec.labels.as_ref().unwrap();
        assert_eq!(labels[0].name, "kind/bug");
    }

    // --- resolve_patch_path ---

    #[test]
    fn test_resolve_patch_path_default() {
        let rule = Rule {
            path: String::from("Cargo.toml"),
            strategy: Strategy::Patch,
            source: None,
            patch: None,
        };
        assert_eq!(
            resolve_patch_path(&rule),
            ".github/gh-sync/patches/Cargo.toml.patch"
        );
    }

    #[test]
    fn test_resolve_patch_path_nested() {
        let rule = Rule {
            path: String::from(".github/workflows/ci.yaml"),
            strategy: Strategy::Patch,
            source: None,
            patch: None,
        };
        assert_eq!(
            resolve_patch_path(&rule),
            ".github/gh-sync/patches/.github/workflows/ci.yaml.patch"
        );
    }

    #[test]
    fn test_resolve_patch_path_explicit() {
        let rule = Rule {
            path: String::from("Cargo.toml"),
            strategy: Strategy::Patch,
            source: None,
            patch: Some(String::from("custom/cargo.patch")),
        };
        assert_eq!(resolve_patch_path(&rule), "custom/cargo.patch");
    }

    // --- Manifest::load ---

    #[test]
    fn test_manifest_load_valid() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yml");
        std::fs::write(
            &path,
            r"
upstream:
  repo: owner/repo
files:
  - path: foo.txt
    strategy: replace
",
        )
        .unwrap();
        let m = Manifest::load(&path).unwrap();
        assert_eq!(m.upstream.repo, "owner/repo");
        assert_eq!(m.upstream.ref_, "main");
        assert_eq!(m.files.len(), 1);
    }

    #[test]
    fn test_manifest_load_missing_file() {
        let path = std::path::Path::new("/nonexistent/config.yml");
        assert!(matches!(
            Manifest::load(path),
            Err(SyncError::ManifestLoad { .. })
        ));
    }

    // --- validate_schema: patterns_allowed ---

    fn expect_schema_error_contains(yaml: &str, needle: &str) {
        let m = manifest_from_yaml(yaml);
        let Err(SyncError::Validation(errors)) = validate_schema(&m) else {
            panic!("expected validation error containing '{needle}', got Ok");
        };
        assert!(
            errors
                .iter()
                .any(|e| e.field.contains(needle) || e.message.contains(needle)),
            "expected error containing '{needle}', got: {errors:?}"
        );
    }

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_schema_pattern_missing_ref_is_error() {
        expect_schema_error_contains(
            r"
upstream:
  repo: owner/repo
files:
  - path: foo.txt
    strategy: replace
spec:
  actions:
    enabled: true
    allowed_actions: selected
    selected_actions:
      patterns_allowed:
        - 'jdx/mise-action'
",
            "no ref specifier",
        );
    }

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_schema_pattern_uppercase_owner_is_error() {
        expect_schema_error_contains(
            r"
upstream:
  repo: owner/repo
files:
  - path: foo.txt
    strategy: replace
spec:
  actions:
    enabled: true
    allowed_actions: selected
    selected_actions:
      patterns_allowed:
        - 'Songmu/tagpr@*'
",
            "uppercase",
        );
    }

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_schema_pattern_with_wildcard_ref_is_ok() {
        expect_schema_ok(
            r"
upstream:
  repo: owner/repo
files:
  - path: foo.txt
    strategy: replace
spec:
  actions:
    enabled: true
    allowed_actions: selected
    selected_actions:
      patterns_allowed:
        - 'jdx/mise-action@*'
        - 'songmu/tagpr@*'
",
        );
    }

    // --- Strategy::Ignore ---

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_schema_ignore_is_valid() {
        expect_schema_ok(
            r"
upstream:
  repo: owner/repo
files:
  - path: foo.txt
    strategy: ignore
",
        );
    }

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_schema_ignore_with_source() {
        expect_schema_error(
            r"
upstream:
  repo: owner/repo
files:
  - path: foo.txt
    strategy: ignore
    source: bar.txt
",
            "source",
        );
    }

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn test_schema_ignore_with_patch() {
        expect_schema_error(
            r"
upstream:
  repo: owner/repo
files:
  - path: foo.txt
    strategy: ignore
    patch: foo.patch
",
            "patch",
        );
    }

    #[test]
    fn test_strategy_display() {
        assert_eq!(Strategy::Replace.to_string(), "replace");
        assert_eq!(Strategy::CreateOnly.to_string(), "create_only");
        assert_eq!(Strategy::Delete.to_string(), "delete");
        assert_eq!(Strategy::Patch.to_string(), "patch");
        assert_eq!(Strategy::Ignore.to_string(), "ignore");
    }

    // --- validate_schema: merge_strategy all-disabled guard ---

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn validate_schema_merge_strategy_all_disabled_is_error() {
        expect_schema_error(
            r"
upstream:
  repo: owner/repo
files:
  - path: foo.txt
    strategy: replace
spec:
  merge_strategy:
    allow_merge_commit: false
    allow_squash_merge: false
    allow_rebase_merge: false
",
            "spec.merge_strategy",
        );
    }

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn validate_schema_merge_strategy_one_enabled_is_ok() {
        expect_schema_ok(
            r"
upstream:
  repo: owner/repo
files:
  - path: foo.txt
    strategy: replace
spec:
  merge_strategy:
    allow_merge_commit: false
    allow_squash_merge: false
    allow_rebase_merge: true
",
        );
    }

    // --- validate_schema: allowed_actions ---

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn validate_schema_allowed_actions_invalid_value_is_error() {
        expect_schema_error(
            r"
upstream:
  repo: owner/repo
files:
  - path: foo.txt
    strategy: replace
spec:
  actions:
    allowed_actions: bad_value
",
            "spec.actions.allowed_actions",
        );
    }

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn validate_schema_allowed_actions_valid_value_is_ok() {
        expect_schema_ok(
            r"
upstream:
  repo: owner/repo
files:
  - path: foo.txt
    strategy: replace
spec:
  actions:
    allowed_actions: all
",
        );
    }

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn validate_schema_allowed_actions_selected_without_patterns_is_error() {
        expect_schema_error(
            r"
upstream:
  repo: owner/repo
files:
  - path: foo.txt
    strategy: replace
spec:
  actions:
    allowed_actions: selected
",
            "spec.actions.selected_actions",
        );
    }

    #[cfg_attr(miri, ignore = "libyml ptr_offset_from UB under Miri")]
    #[test]
    fn validate_schema_allowed_actions_selected_with_patterns_is_ok() {
        expect_schema_ok(
            r"
upstream:
  repo: owner/repo
files:
  - path: foo.txt
    strategy: replace
spec:
  actions:
    allowed_actions: selected
    selected_actions:
      github_owned_allowed: true
",
        );
    }

    #[cfg_attr(miri, ignore = "tempfile I/O not supported under Miri")]
    #[test]
    fn test_manifest_load_unknown_field() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yml");
        std::fs::write(
            &path,
            r"
upstream:
  repo: owner/repo
  unknown_field: bad
files:
  - path: foo.txt
    strategy: replace
",
        )
        .unwrap();
        assert!(matches!(
            Manifest::load(&path),
            Err(SyncError::ManifestLoad { .. })
        ));
    }
}
