//! Re-export error types from `gh-sync-manifest` so internal modules keep existing import paths.
#![allow(unused_imports)]
#[allow(clippy::module_name_repetitions)]
pub use gh_sync_manifest::error::{SyncError, ValidationError};
