//! Business logic, traits, and output formatting for `gh-sync`.
//!
//! This crate contains no I/O against external binaries (`gh`, `patch`, `diff`)
//! and is suitable for Miri testing. Production I/O implementations live in the
//! `gh-sync` binary crate.

/// Generate unified diffs between byte slices (spawns `diff` — Miri-ignored).
pub mod diff;
/// Sync operation modes: validate, sync, ci-check, patch-refresh.
pub mod mode;
/// Formatted terminal and PR output for sync operations.
pub mod output;
/// Repository client trait, API types, and pure comparison/apply logic.
pub mod repo;
/// File sync strategy implementations.
pub mod strategy;
/// Upstream fetcher trait and associated types.
pub mod upstream;
