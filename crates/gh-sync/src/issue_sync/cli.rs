/// CLI argument definitions for the `issue-sync` subcommand.
use std::path::PathBuf;

use clap::Parser;

/// Arguments for `gh-sync issue-sync`.
#[derive(Debug, Parser)]
pub struct IssueSyncArgs {
    /// Path to the local gh-sync manifest file.
    #[arg(long, default_value = ".github/gh-sync/config.yaml")]
    pub manifest: PathBuf,

    /// Upstream manifest reference in `owner/repo@ref:path` format.
    ///
    /// When provided, the upstream manifest is fetched and merged with the local
    /// manifest (if present).
    #[arg(long)]
    pub upstream_manifest: Option<String>,

    /// Target GitHub repository in `owner/repo` format.
    ///
    /// When omitted, the repository is detected via `gh repo view`.
    #[arg(long)]
    pub repo: Option<String>,

    /// Issue label used to identify the tracking drift issue.
    #[arg(long, default_value = "gh-sync-drift")]
    pub label: String,

    /// Title for the drift tracking issue.
    #[arg(long, default_value = "chore(gh-sync): upstream drift detected")]
    pub title: String,
}
