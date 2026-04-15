//! Pure data types, validation, and schema for `gh-sync` manifests.
//!
//! This crate contains no I/O beyond reading YAML files and is fully
//! compatible with Miri.

/// Error types for manifest validation and sync operations.
pub mod error;
/// Manifest schema types, loading, and validation.
pub mod manifest;
/// Upstream manifest + local overlay merge logic.
pub mod merge;
/// Strategy outcome type.
pub mod strategy;

pub use error::{SyncError, ValidationError};
pub use manifest::{
    Actions, BranchProtection, BranchProtectionStatusChecks, BypassActor, Features, Label,
    Manifest, MergeStrategy, PullRequestRule, RefNameCondition, RequiredStatusChecks, Rule,
    Ruleset, RulesetConditions, RulesetRules, SelectedActions, Spec, StatusCheckContext, Strategy,
    Upstream, resolve_patch_path, validate_references, validate_schema,
};
pub use merge::merge_overlay;
pub use strategy::StrategyResult;
