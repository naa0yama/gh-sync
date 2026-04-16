/// CLI argument definitions for the `pr` subcommand.
#[derive(clap::Parser, Debug)]
pub struct PrArgs {
    /// Pull request title
    #[arg(long = "title")]
    pub title: String,

    /// Pull request body
    #[arg(long = "body", default_value = "")]
    pub body: String,

    /// Branch name prefix; a timestamp suffix is appended automatically
    #[arg(long = "branch-prefix", default_value = "gh-sync/auto")]
    pub branch_prefix: String,

    /// Base branch for the pull request (defaults to the repository default branch)
    #[arg(long = "base")]
    pub base: Option<String>,

    /// Commit message written into the signed commit
    #[arg(long = "commit-message")]
    pub commit_message: Option<String>,
}
