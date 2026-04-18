use std::path::PathBuf;

use clap::{ArgGroup, Parser};

/// Arguments for the `init` subcommand.
#[derive(Parser, Debug)]
#[command(group(
    ArgGroup::new("mode")
        .required(true)
        .args(["upstream", "downstream"])
))]
#[allow(clippy::struct_excessive_bools)]
pub struct InitArgs {
    /// Generate config.yaml + schema.json for this template (upstream) project
    #[arg(long)]
    pub upstream: bool,

    /// Generate workflow + optional Claude skill for this downstream project
    #[arg(long)]
    pub downstream: bool,

    /// Repository in `owner/name` format.
    /// For --upstream: this project's own repo (stored in config upstream.repo).
    /// For --downstream: the upstream template repo to sync from.
    #[arg(short = 'r', long = "repo")]
    pub repo: Option<String>,

    /// Git ref to use (branch, tag, or commit SHA)
    #[arg(long = "ref", default_value = "main")]
    pub ref_: String,

    /// Output path for the generated config file (--upstream only)
    #[arg(short = 'o', long = "output", conflicts_with = "downstream")]
    pub output: Option<PathBuf>,

    /// Interactively select files from the repository (--upstream only)
    #[arg(long = "select", conflicts_with = "downstream")]
    pub select: bool,

    /// Also generate a Claude Code skill file for marker usage (--downstream only)
    #[arg(long = "with-skill", conflicts_with = "upstream")]
    pub with_skill: bool,

    /// Overwrite existing files without prompting
    #[arg(long = "force")]
    pub force: bool,
}
