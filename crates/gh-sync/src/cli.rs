use clap::{Parser, Subcommand};

use crate::init::cli::InitArgs;
use crate::sync::cli::SyncArgs;

/// gh-sync CLI for pulling upstream files into downstream repos.
#[derive(Parser, Debug)]
#[command(about, version = crate::APP_VERSION)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

/// Available subcommands.
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Synchronize files or repository settings from upstream
    Sync(SyncArgs),
    /// Initialize a gh-sync configuration file
    Init(InitArgs),
}
