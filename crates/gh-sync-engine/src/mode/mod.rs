/// `--ci-check` mode: drift detection with optional GHA annotations.
pub mod ci_check;
/// `--patch-refresh` mode: regenerate patch files from upstream diff.
pub mod patch_refresh;
/// Default sync mode: fetch upstream, preview changes, then apply on confirmation.
pub mod sync;
/// `--validate` mode: offline manifest schema and reference validation.
pub mod validate;
