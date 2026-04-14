//! Re-export manifest types from `gh-sync-manifest` so internal modules keep existing import paths.
#![allow(unused_imports)]
pub use gh_sync_manifest::manifest::{
    Actions, BranchProtection, BranchProtectionStatusChecks, BypassActor, Features, Label,
    Manifest, MergeStrategy, PullRequestRule, RefNameCondition, RequiredStatusChecks, Rule,
    Ruleset, RulesetConditions, RulesetRules, SelectedActions, Spec, StatusCheckContext, Strategy,
    Upstream, resolve_patch_path, validate_references, validate_schema,
};
