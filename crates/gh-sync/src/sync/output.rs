//! Re-export output helpers from `gh-sync-engine` so internal modules keep existing import paths.
#![allow(unused_imports)]
pub use gh_sync_engine::output::{
    DriftOutcome, DriftSummary, RuleOutcome, StatusTag, Summary, build_pr_comment, colorize_diff,
    emit_diff, emit_drift_summary, emit_gha_annotations, emit_status, emit_summary,
};
