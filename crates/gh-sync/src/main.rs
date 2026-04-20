//! gh-sync — template sync CLI for pulling upstream files into downstream repos.
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

mod cli;
mod init;
mod issue_sync;
mod sync;

use std::process::ExitCode;

use clap::Parser;
use tracing_subscriber::filter::EnvFilter;

use cli::{Cli, Commands};

const APP_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (rev:", env!("GIT_HASH"), ")",);

#[cfg_attr(coverage_nightly, coverage(off))]
fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    match cli.command {
        Commands::Sync(args) => sync::execute(&args),
        Commands::Init(args) => init::execute(&args),
        Commands::IssueSync(args) => issue_sync::execute(&args),
    }
}
