//! Re-export diff utilities from `gh-sync-engine` so internal modules keep existing import paths.
#![allow(unused_imports)]
#[allow(clippy::module_name_repetitions)]
pub use gh_sync_engine::diff::unified_diff;
