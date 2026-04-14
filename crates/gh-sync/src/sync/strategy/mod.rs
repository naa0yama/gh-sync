/// `create_only` strategy: create file only if it does not exist locally
pub mod create_only;
/// `delete` strategy: explicitly remove a local file or directory
pub mod delete;
/// `patch` strategy: apply a unified diff on top of upstream content
pub mod patch;
/// `replace` strategy: overwrite local file with upstream content
pub mod replace;

// Re-export StrategyResult from the manifest crate.
#[allow(unused_imports, clippy::module_name_repetitions)]
pub use gh_sync_manifest::strategy::StrategyResult;
