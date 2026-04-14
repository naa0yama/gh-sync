use std::path::PathBuf;

use clap::Parser;

/// Arguments for the `init` subcommand.
#[derive(Parser, Debug)]
pub struct InitArgs {
    /// Upstream repository in `owner/name` format
    #[arg(short = 'r', long = "repo")]
    pub repo: Option<String>,

    /// Git ref to use (branch, tag, or commit SHA)
    #[arg(long = "ref", default_value = "main")]
    pub ref_: String,

    /// Output path for the generated configuration file
    #[arg(
        short = 'o',
        long = "output",
        default_value = ".github/gh-sync/config.yaml"
    )]
    pub output: PathBuf,

    /// Copy the upstream's own gh-sync config (non-interactive)
    #[arg(long = "from-upstream", conflicts_with = "select")]
    pub from_upstream: bool,

    /// Interactively select files from the upstream repository
    #[arg(long = "select", conflicts_with = "from_upstream")]
    pub select: bool,

    /// Overwrite an existing config file without prompting
    #[arg(long = "force")]
    pub force: bool,
}
