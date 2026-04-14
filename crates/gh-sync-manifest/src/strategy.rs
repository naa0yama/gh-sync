/// Outcome of applying a synchronisation strategy to a single rule.
#[derive(Debug)]
#[allow(clippy::module_name_repetitions)] // "StrategyResult" in module "strategy" is intentional
pub enum StrategyResult {
    /// File content changed; `content` is the new bytes to write.
    Changed {
        /// New file contents to write to disk.
        content: Vec<u8>,
    },
    /// Local file already matches the expected state — no write needed.
    Unchanged,
    /// Rule was intentionally not applied (e.g. `create_only` with existing file).
    Skipped {
        /// Human-readable explanation of why the rule was skipped.
        reason: String,
    },
    /// File should be deleted (returned by the `delete` strategy).
    Deleted,
    /// Patch application conflicted; the file was NOT modified.
    Conflict {
        /// Human-readable description of the conflict.
        message: String,
    },
    /// Rule application failed (non-recoverable).
    Error(String),
}
