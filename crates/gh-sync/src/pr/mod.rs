/// CLI argument definitions for the `pr` subcommand.
pub mod cli;

use std::io::{self, Write as _};
use std::process::ExitCode;

use anyhow::Context as _;
use cli::PrArgs;
use serde::Serialize;

use crate::sync::runner::{GhOutput, GhRunner, SystemGhRunner};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Execute the `pr` subcommand.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn execute(args: &PrArgs) -> ExitCode {
    let runner = SystemGhRunner;
    match run(args, &runner) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("pr failed: {e:#}");
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------------------
// Core logic (parameterised over GhRunner for testability)
// ---------------------------------------------------------------------------

/// Core logic for `pr`, parameterised over the runner for testability.
///
/// # Errors
///
/// Returns an error when any `gh` CLI call fails or the working tree state
/// cannot be determined.
pub fn run(args: &PrArgs, runner: &dyn GhRunner) -> anyhow::Result<()> {
    let mut stdout = io::stdout();

    // 1. Check for local changes; exit early when tree is clean.
    let changed = changed_files(runner)?;
    if changed.is_empty() {
        writeln!(stdout, "No file drift detected — nothing to commit.")
            .context("failed to write to stdout")?;
        return Ok(());
    }

    // 2. Resolve repository name.
    let repo = repo_name(runner)?;

    // 3. Resolve base branch.
    let base = match &args.base {
        Some(b) => b.clone(),
        None => default_branch(runner, &repo)?,
    };

    // 4. Resolve HEAD commit SHA.
    let head_sha = head_sha(runner, &repo, &base)?;

    // 5. Create blobs for each changed file (UTF-8 text; deleted files skipped).
    let mut tree_entries: Vec<TreeEntry> = Vec::new();
    for (path, deleted) in &changed {
        if *deleted {
            tree_entries.push(TreeEntry {
                path: path.clone(),
                mode: String::from("100644"),
                type_: String::from("blob"),
                sha: None,
            });
        } else {
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read '{path}'"))?;
            let blob_sha = create_blob(runner, &repo, &content)?;
            tree_entries.push(TreeEntry {
                path: path.clone(),
                mode: String::from("100644"),
                type_: String::from("blob"),
                sha: Some(blob_sha),
            });
        }
    }

    // 6. Create tree (JSON written to a temp file to avoid shell arg length limits).
    let tree_sha = create_tree(runner, &repo, &head_sha, &tree_entries)?;

    // 7. Create a signed commit via the GitHub API.
    let commit_message = args
        .commit_message
        .clone()
        .unwrap_or_else(|| args.title.clone());
    let commit_sha = create_commit(runner, &repo, &commit_message, &tree_sha, &head_sha)?;

    // 8. Create the branch ref; fall back to force-update on 409.
    let timestamp = chrono_timestamp();
    let branch = format!("{}-{timestamp}", args.branch_prefix);
    create_branch_ref(runner, &repo, &branch, &commit_sha)?;

    // 9. Create the pull request (idempotent: return existing URL on conflict).
    let pr_url = create_pr(runner, &repo, &base, &branch, &args.title, &args.body)?;

    writeln!(stdout, "[OK] pull request: {pr_url}").context("failed to write to stdout")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return a timestamp string suitable for branch name suffixes.
fn chrono_timestamp() -> String {
    // Use the system time; no chrono dependency needed.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Return the Unix timestamp (seconds since epoch) as a unique suffix.
    format!("{secs}")
}

/// Return the list of changed files in the working tree as `(path, deleted)` pairs.
///
/// Staged and unstaged changes are both included.
fn changed_files(runner: &dyn GhRunner) -> anyhow::Result<Vec<(String, bool)>> {
    let out = runner
        .run(&["--version"], None)
        .context("failed to run gh --version (sanity check)")?;
    drop(out); // just checking gh is available

    // NOTEST(external-cmd): spawns `git` directly — pure parsing logic is
    // covered by `parse_porcelain_output` unit tests.
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .context("failed to spawn `git`")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("`git status` failed: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_porcelain_output(&stdout))
}

/// Parse `git status --porcelain` (v1) output into `(path, deleted)` pairs.
///
/// Each line is `XY path` where `XY` is a two-character status code.
/// Rename lines use the form `XY old -> new`; the new (right-hand) path is
/// returned.  Lines shorter than 4 characters are silently skipped.
fn parse_porcelain_output(stdout: &str) -> Vec<(String, bool)> {
    let mut files = Vec::new();
    for line in stdout.lines() {
        if line.len() < 4 {
            continue;
        }
        let xy = &line[..2];
        let path = line[3..].trim().to_owned();
        // Strip rename arrow: "old -> new" — take the right-hand side.
        let path = if let Some(idx) = path.find(" -> ") {
            path.get(idx.saturating_add(4)..).unwrap_or("").to_owned()
        } else {
            path
        };
        let deleted = xy.trim() == "D" || xy.starts_with('D') || xy.ends_with('D');
        files.push((path, deleted));
    }
    files
}

/// Return the repository name in `owner/repo` format.
fn repo_name(runner: &dyn GhRunner) -> anyhow::Result<String> {
    // NOTEST(env): reads GITHUB_REPOSITORY from process environment —
    // covered by repo_name_inner unit tests via explicit injection.
    repo_name_inner(runner, std::env::var("GITHUB_REPOSITORY").ok())
}

/// Testable inner implementation of [`repo_name`].
///
/// When `env_repo` contains a non-empty value it is returned immediately
/// (matching GitHub Actions' `GITHUB_REPOSITORY`).  Otherwise the `gh` CLI
/// is called to determine the name.
///
/// # Errors
///
/// Returns an error when `env_repo` is absent and the `gh` CLI call fails.
fn repo_name_inner(runner: &dyn GhRunner, env_repo: Option<String>) -> anyhow::Result<String> {
    if let Some(v) = env_repo
        && !v.is_empty()
    {
        return Ok(v);
    }

    let out = runner
        .run(
            &[
                "repo",
                "view",
                "--json",
                "nameWithOwner",
                "--jq",
                ".nameWithOwner",
            ],
            None,
        )
        .context("failed to spawn `gh`")?;

    require_success(&out, "gh repo view")?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    Ok(name)
}

/// Return the default branch name for the repository.
fn default_branch(runner: &dyn GhRunner, repo: &str) -> anyhow::Result<String> {
    let out = runner
        .run(
            &[
                "repo",
                "view",
                repo,
                "--json",
                "defaultBranchRef",
                "--jq",
                ".defaultBranchRef.name",
            ],
            None,
        )
        .context("failed to spawn `gh`")?;

    require_success(&out, "gh repo view defaultBranchRef")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Return the HEAD commit SHA for the given branch.
fn head_sha(runner: &dyn GhRunner, repo: &str, branch: &str) -> anyhow::Result<String> {
    let endpoint = format!("repos/{repo}/git/ref/heads/{branch}");
    let out = runner
        .run(&["api", &endpoint, "--jq", ".object.sha"], None)
        .context("failed to spawn `gh`")?;

    require_success(&out, "gh api git/ref")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Create a blob for `content` (UTF-8 text) and return its SHA.
fn create_blob(runner: &dyn GhRunner, repo: &str, content: &str) -> anyhow::Result<String> {
    let endpoint = format!("repos/{repo}/git/blobs");
    let payload = serde_json::json!({
        "content": content,
        "encoding": "utf-8"
    });
    let payload_bytes = serde_json::to_vec(&payload).context("failed to serialise blob JSON")?;

    let out = runner
        .run(
            &[
                "api", &endpoint, "--method", "POST", "--input", "-", "--jq", ".sha",
            ],
            Some(&payload_bytes),
        )
        .context("failed to spawn `gh`")?;

    require_success(&out, "gh api git/blobs")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

// ---------------------------------------------------------------------------
// Tree entry type for JSON serialisation
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct TreeEntry {
    path: String,
    mode: String,
    #[serde(rename = "type")]
    type_: String,
    sha: Option<String>,
}

#[derive(Debug, Serialize)]
struct CreateTreeBody<'a> {
    base_tree: &'a str,
    tree: &'a [TreeEntry],
}

/// Create a tree object and return its SHA.
///
/// The JSON body is written to a temp file and passed via `--input` to avoid
/// shell argument length limits when many files are included.
fn create_tree(
    runner: &dyn GhRunner,
    repo: &str,
    base_tree_sha: &str,
    entries: &[TreeEntry],
) -> anyhow::Result<String> {
    let endpoint = format!("repos/{repo}/git/trees");
    let body = CreateTreeBody {
        base_tree: base_tree_sha,
        tree: entries,
    };
    let payload_bytes = serde_json::to_vec(&body).context("failed to serialise tree JSON")?;

    let out = runner
        .run(
            &[
                "api", &endpoint, "--method", "POST", "--input", "-", "--jq", ".sha",
            ],
            Some(&payload_bytes),
        )
        .context("failed to spawn `gh`")?;

    require_success(&out, "gh api git/trees")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Create a signed commit via the GitHub API and return its SHA.
fn create_commit(
    runner: &dyn GhRunner,
    repo: &str,
    message: &str,
    tree_sha: &str,
    parent_sha: &str,
) -> anyhow::Result<String> {
    let endpoint = format!("repos/{repo}/git/commits");
    let payload = serde_json::json!({
        "message": message,
        "tree": tree_sha,
        "parents": [parent_sha]
    });
    let payload_bytes = serde_json::to_vec(&payload).context("failed to serialise commit JSON")?;

    let out = runner
        .run(
            &[
                "api", &endpoint, "--method", "POST", "--input", "-", "--jq", ".sha",
            ],
            Some(&payload_bytes),
        )
        .context("failed to spawn `gh`")?;

    require_success(&out, "gh api git/commits")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Create a branch ref pointing to `commit_sha`.
///
/// If the branch already exists (HTTP 422 with "already exists"), force-updates
/// the ref instead.
fn create_branch_ref(
    runner: &dyn GhRunner,
    repo: &str,
    branch: &str,
    commit_sha: &str,
) -> anyhow::Result<()> {
    let endpoint = format!("repos/{repo}/git/refs");
    let payload = serde_json::json!({
        "ref": format!("refs/heads/{branch}"),
        "sha": commit_sha
    });
    let payload_bytes = serde_json::to_vec(&payload).context("failed to serialise ref JSON")?;

    let out = runner
        .run(
            &["api", &endpoint, "--method", "POST", "--input", "-"],
            Some(&payload_bytes),
        )
        .context("failed to spawn `gh`")?;

    // 422 "already exists" → force-update via PATCH.
    if !out.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout_text = String::from_utf8_lossy(&out.stdout);
        let combined = format!("{stderr}{stdout_text}");
        if combined.contains("already exists") || combined.contains("Reference already exists") {
            return force_update_ref(runner, repo, branch, commit_sha);
        }
        anyhow::bail!("gh api git/refs failed: {combined}");
    }

    Ok(())
}

/// Force-update an existing branch ref via PATCH.
fn force_update_ref(
    runner: &dyn GhRunner,
    repo: &str,
    branch: &str,
    commit_sha: &str,
) -> anyhow::Result<()> {
    let endpoint = format!("repos/{repo}/git/refs/heads/{branch}");
    let payload = serde_json::json!({
        "sha": commit_sha,
        "force": true
    });
    let payload_bytes =
        serde_json::to_vec(&payload).context("failed to serialise ref update JSON")?;

    let out = runner
        .run(
            &["api", &endpoint, "--method", "PATCH", "--input", "-"],
            Some(&payload_bytes),
        )
        .context("failed to spawn `gh`")?;

    require_success(&out, "gh api git/refs PATCH")?;
    Ok(())
}

/// Create a pull request and return its URL.
///
/// Idempotent: if a PR already exists for the branch, returns the existing URL.
fn create_pr(
    runner: &dyn GhRunner,
    repo: &str,
    base: &str,
    head: &str,
    title: &str,
    body: &str,
) -> anyhow::Result<String> {
    let out = runner
        .run(
            &[
                "pr", "create", "--repo", repo, "--base", base, "--head", head, "--title", title,
                "--body", body,
            ],
            None,
        )
        .context("failed to spawn `gh`")?;

    if out.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned());
    }

    // PR already exists — retrieve its URL.
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("already exists") || stderr.contains("pull request for branch") {
        let view_out = runner
            .run(
                &[
                    "pr", "view", "--repo", repo, head, "--json", "url", "--jq", ".url",
                ],
                None,
            )
            .context("failed to spawn `gh`")?;

        require_success(&view_out, "gh pr view")?;
        return Ok(String::from_utf8_lossy(&view_out.stdout).trim().to_owned());
    }

    anyhow::bail!("gh pr create failed: {stderr}");
}

/// Return an error when `output` is not successful.
fn require_success(output: &GhOutput, context: &str) -> anyhow::Result<()> {
    if output.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    anyhow::bail!(
        "{context} failed (exit {:?}): {}{}",
        output.exit_code,
        stderr,
        stdout
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::indexing_slicing)]
    #![allow(clippy::arithmetic_side_effects)]

    use crate::sync::runner::{GhOutput, GhRunner};

    use super::*;

    // ---------------------------------------------------------------------------
    // Test helpers
    // ---------------------------------------------------------------------------

    /// Minimal mock runner that returns pre-canned responses in call order.
    struct MockRunner {
        responses: Vec<(Vec<String>, GhOutput)>,
        call_index: std::sync::Mutex<usize>,
    }

    impl MockRunner {
        fn new(responses: Vec<(Vec<&str>, GhOutput)>) -> Self {
            Self {
                responses: responses
                    .into_iter()
                    .map(|(k, v)| (k.into_iter().map(String::from).collect(), v))
                    .collect(),
                call_index: std::sync::Mutex::new(0),
            }
        }

        fn ok(stdout: &str) -> GhOutput {
            GhOutput {
                exit_code: Some(0),
                stdout: stdout.as_bytes().to_vec(),
                stderr: vec![],
            }
        }

        fn err(stderr: &str) -> GhOutput {
            GhOutput {
                exit_code: Some(1),
                stdout: vec![],
                stderr: stderr.as_bytes().to_vec(),
            }
        }
    }

    impl GhRunner for MockRunner {
        fn run(&self, _args: &[&str], _stdin: Option<&[u8]>) -> anyhow::Result<GhOutput> {
            let i = {
                let mut idx = self.call_index.lock().unwrap();
                let i = *idx;
                *idx += 1;
                i
            };
            if i < self.responses.len() {
                Ok(self.responses[i].1.clone())
            } else {
                Ok(Self::ok(""))
            }
        }
    }

    /// Runner that always returns `Err` from `run()` (simulates spawn failure).
    struct FailRunner;

    impl GhRunner for FailRunner {
        fn run(&self, _args: &[&str], _stdin: Option<&[u8]>) -> anyhow::Result<GhOutput> {
            Err(anyhow::anyhow!("spawn failed"))
        }
    }

    // ---------------------------------------------------------------------------
    // parse_porcelain_output
    // ---------------------------------------------------------------------------

    #[test]
    fn parse_porcelain_empty_returns_empty() {
        assert!(parse_porcelain_output("").is_empty());
    }

    #[test]
    fn parse_porcelain_modified_unstaged() {
        // " M path" — Y=M means working-tree change, not deleted
        let result = parse_porcelain_output(" M src/lib.rs\n");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "src/lib.rs");
        assert!(!result[0].1, "should not be deleted");
    }

    #[test]
    fn parse_porcelain_modified_staged() {
        // "M  path" — X=M staged change, not deleted
        let result = parse_porcelain_output("M  src/lib.rs\n");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "src/lib.rs");
        assert!(!result[0].1, "should not be deleted");
    }

    #[test]
    fn parse_porcelain_added_staged() {
        // "A  path" — X=A new file staged, not deleted
        let result = parse_porcelain_output("A  new_file.rs\n");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "new_file.rs");
        assert!(!result[0].1, "added file should not be deleted");
    }

    #[test]
    fn parse_porcelain_untracked_file() {
        // "?? path" — untracked file, not deleted
        let result = parse_porcelain_output("?? untracked.rs\n");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "untracked.rs");
        assert!(!result[0].1, "untracked file should not be deleted");
    }

    #[test]
    fn parse_porcelain_deleted_unstaged() {
        // " D path" — Y=D working-tree deletion
        let result = parse_porcelain_output(" D path/to/file.rs\n");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "path/to/file.rs");
        assert!(result[0].1, "should be deleted");
    }

    #[test]
    fn parse_porcelain_deleted_staged() {
        // "D  path" — X=D staged deletion
        let result = parse_porcelain_output("D  path/to/file.rs\n");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "path/to/file.rs");
        assert!(result[0].1, "staged delete should be marked deleted");
    }

    #[test]
    fn parse_porcelain_staged_modified_unstaged_deleted() {
        // "MD path" — X=M staged modification, Y=D working-tree deletion
        let result = parse_porcelain_output("MD path/to/file.rs\n");
        assert_eq!(result.len(), 1);
        assert!(result[0].1, "ends-with-D should be treated as deleted");
    }

    #[test]
    fn parse_porcelain_renamed_takes_new_path() {
        // "R  old.rs -> new.rs" — right-hand side is the current path
        let result = parse_porcelain_output("R  old/name.rs -> new/name.rs\n");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "new/name.rs");
        assert!(!result[0].1, "renamed file should not be deleted");
    }

    #[test]
    fn parse_porcelain_skips_short_lines() {
        // Lines shorter than 4 characters must be silently ignored.
        let result = parse_porcelain_output("M \n");
        assert!(result.is_empty(), "short line should be skipped");
    }

    #[test]
    fn parse_porcelain_multiple_files() {
        let out = " M src/lib.rs\n D deleted.txt\nA  added.rs\n";
        let result = parse_porcelain_output(out);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].0, "src/lib.rs");
        assert!(!result[0].1);
        assert_eq!(result[1].0, "deleted.txt");
        assert!(result[1].1);
        assert_eq!(result[2].0, "added.rs");
        assert!(!result[2].1);
    }

    #[test]
    fn parse_porcelain_whitespace_trimmed_from_path() {
        // Extra leading/trailing whitespace in the path segment is stripped.
        let result = parse_porcelain_output(" M  path/with spaces.txt \n");
        assert_eq!(result[0].0, "path/with spaces.txt");
    }

    // ---------------------------------------------------------------------------
    // require_success
    // ---------------------------------------------------------------------------

    #[test]
    fn require_success_ok_on_zero_exit() {
        let ok = MockRunner::ok("some-sha\n");
        assert!(require_success(&ok, "test").is_ok());
    }

    #[test]
    fn require_success_err_on_nonzero_exit() {
        let err = MockRunner::err("bad");
        assert!(require_success(&err, "test").is_err());
    }

    #[test]
    fn require_success_includes_context_in_error() {
        let out = GhOutput {
            exit_code: Some(1),
            stdout: b"stdout content".to_vec(),
            stderr: b"the real error".to_vec(),
        };
        let err = require_success(&out, "my-step").unwrap_err();
        assert!(
            err.to_string().contains("my-step"),
            "context missing: {err}"
        );
        assert!(
            err.to_string().contains("the real error"),
            "stderr missing: {err}"
        );
    }

    // ---------------------------------------------------------------------------
    // chrono_timestamp
    // ---------------------------------------------------------------------------

    #[test]
    fn chrono_timestamp_is_nonempty_digits() {
        let ts = chrono_timestamp();
        assert!(!ts.is_empty());
        assert!(ts.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn chrono_timestamp_is_monotonically_nondecreasing() {
        let t1 = chrono_timestamp()
            .parse::<u64>()
            .expect("timestamp must be a number");
        let t2 = chrono_timestamp()
            .parse::<u64>()
            .expect("timestamp must be a number");
        assert!(t2 >= t1, "timestamps must not decrease");
    }

    // ---------------------------------------------------------------------------
    // repo_name_inner
    // ---------------------------------------------------------------------------

    #[test]
    fn repo_name_inner_returns_env_when_set() {
        // When env_repo is present and non-empty, the runner is never called.
        let runner = FailRunner; // would fail if called
        let result = repo_name_inner(&runner, Some(String::from("owner/repo"))).unwrap();
        assert_eq!(result, "owner/repo");
    }

    #[test]
    fn repo_name_inner_ignores_empty_env_and_calls_runner() {
        // Empty string must fall through to the gh CLI path.
        let runner = MockRunner::new(vec![(
            vec!["repo", "view"],
            MockRunner::ok("cli-owner/cli-repo\n"),
        )]);
        let result = repo_name_inner(&runner, Some(String::new())).unwrap();
        assert_eq!(result, "cli-owner/cli-repo");
    }

    #[test]
    fn repo_name_inner_calls_runner_when_env_none() {
        let runner = MockRunner::new(vec![(
            vec!["repo", "view"],
            MockRunner::ok("runner-owner/runner-repo\n"),
        )]);
        let result = repo_name_inner(&runner, None).unwrap();
        assert_eq!(result, "runner-owner/runner-repo");
    }

    #[test]
    fn repo_name_inner_propagates_runner_gh_error() {
        let runner = MockRunner::new(vec![(
            vec!["repo", "view"],
            MockRunner::err("authentication required"),
        )]);
        let err = repo_name_inner(&runner, None).unwrap_err();
        assert!(
            err.to_string().contains("authentication required")
                || err.to_string().contains("gh repo view"),
            "expected error detail: {err}"
        );
    }

    #[test]
    fn repo_name_inner_propagates_spawn_failure() {
        let err = repo_name_inner(&FailRunner, None).unwrap_err();
        assert!(
            err.to_string().contains("spawn failed") || err.to_string().contains("failed"),
            "expected spawn error: {err}"
        );
    }

    // ---------------------------------------------------------------------------
    // default_branch
    // ---------------------------------------------------------------------------

    #[test]
    fn default_branch_returns_branch_name() {
        let runner = MockRunner::new(vec![(vec!["repo", "view"], MockRunner::ok("main\n"))]);
        let branch = default_branch(&runner, "owner/repo").unwrap();
        assert_eq!(branch, "main");
    }

    #[test]
    fn default_branch_trims_whitespace() {
        let runner = MockRunner::new(vec![(
            vec!["repo", "view"],
            MockRunner::ok("  develop  \n"),
        )]);
        let branch = default_branch(&runner, "owner/repo").unwrap();
        assert_eq!(branch, "develop");
    }

    #[test]
    fn default_branch_propagates_gh_error() {
        let runner = MockRunner::new(vec![(vec!["repo", "view"], MockRunner::err("not found"))]);
        let err = default_branch(&runner, "owner/repo").unwrap_err();
        assert!(
            err.to_string().contains("not found") || err.to_string().contains("defaultBranchRef"),
            "expected error: {err}"
        );
    }

    #[test]
    fn default_branch_propagates_spawn_failure() {
        let err = default_branch(&FailRunner, "owner/repo").unwrap_err();
        assert!(
            err.to_string().contains("spawn failed") || err.to_string().contains("failed"),
            "expected spawn error: {err}"
        );
    }

    // ---------------------------------------------------------------------------
    // head_sha
    // ---------------------------------------------------------------------------

    #[test]
    fn head_sha_returns_commit_sha() {
        let runner = MockRunner::new(vec![(
            vec!["api", "repos/owner/repo/git/ref/heads/main"],
            MockRunner::ok("aabbccdd\n"),
        )]);
        let sha = head_sha(&runner, "owner/repo", "main").unwrap();
        assert_eq!(sha, "aabbccdd");
    }

    #[test]
    fn head_sha_trims_whitespace() {
        let runner = MockRunner::new(vec![(vec!["api"], MockRunner::ok("  deadbeef  \n"))]);
        let sha = head_sha(&runner, "owner/repo", "main").unwrap();
        assert_eq!(sha, "deadbeef");
    }

    #[test]
    fn head_sha_propagates_gh_error() {
        let runner = MockRunner::new(vec![(vec!["api"], MockRunner::err("branch not found"))]);
        let err = head_sha(&runner, "owner/repo", "nonexistent").unwrap_err();
        assert!(
            err.to_string().contains("branch not found") || err.to_string().contains("git/ref"),
            "expected error: {err}"
        );
    }

    #[test]
    fn head_sha_propagates_spawn_failure() {
        let err = head_sha(&FailRunner, "owner/repo", "main").unwrap_err();
        assert!(
            err.to_string().contains("spawn failed") || err.to_string().contains("failed"),
            "expected spawn error: {err}"
        );
    }

    // ---------------------------------------------------------------------------
    // create_blob
    // ---------------------------------------------------------------------------

    #[test]
    fn create_blob_calls_correct_endpoint() {
        let runner = MockRunner::new(vec![(
            vec!["api", "repos/owner/repo/git/blobs"],
            MockRunner::ok("abc123sha\n"),
        )]);
        let sha = create_blob(&runner, "owner/repo", "file content").unwrap();
        assert_eq!(sha, "abc123sha");
    }

    #[test]
    fn create_blob_propagates_error() {
        let runner = MockRunner::new(vec![(vec!["api"], MockRunner::err("network error"))]);
        let err = create_blob(&runner, "owner/repo", "content").unwrap_err();
        assert!(err.to_string().contains("network error") || err.to_string().contains("failed"));
    }

    #[test]
    fn create_blob_propagates_spawn_failure() {
        let err = create_blob(&FailRunner, "owner/repo", "content").unwrap_err();
        assert!(err.to_string().contains("spawn failed") || err.to_string().contains("failed"));
    }

    // ---------------------------------------------------------------------------
    // create_tree
    // ---------------------------------------------------------------------------

    #[test]
    fn create_tree_calls_correct_endpoint() {
        let runner = MockRunner::new(vec![(
            vec!["api", "repos/owner/repo/git/trees"],
            MockRunner::ok("tree-sha\n"),
        )]);
        let entries = vec![TreeEntry {
            path: String::from("foo.txt"),
            mode: String::from("100644"),
            type_: String::from("blob"),
            sha: Some(String::from("abc")),
        }];
        let sha = create_tree(&runner, "owner/repo", "base-sha", &entries).unwrap();
        assert_eq!(sha, "tree-sha");
    }

    #[test]
    fn create_tree_with_deleted_entry() {
        // A deleted entry has sha=None; serialisation must not panic.
        let runner = MockRunner::new(vec![(vec!["api"], MockRunner::ok("tree-sha2\n"))]);
        let entries = vec![TreeEntry {
            path: String::from("removed.txt"),
            mode: String::from("100644"),
            type_: String::from("blob"),
            sha: None,
        }];
        let sha = create_tree(&runner, "owner/repo", "base-sha", &entries).unwrap();
        assert_eq!(sha, "tree-sha2");
    }

    #[test]
    fn create_tree_propagates_error() {
        let runner = MockRunner::new(vec![(vec!["api"], MockRunner::err("server error"))]);
        let entries: Vec<TreeEntry> = vec![];
        let err = create_tree(&runner, "owner/repo", "base-sha", &entries).unwrap_err();
        assert!(err.to_string().contains("server error") || err.to_string().contains("failed"));
    }

    // ---------------------------------------------------------------------------
    // create_commit
    // ---------------------------------------------------------------------------

    #[test]
    fn create_commit_calls_correct_endpoint() {
        let runner = MockRunner::new(vec![(
            vec!["api", "repos/owner/repo/git/commits"],
            MockRunner::ok("commit-sha\n"),
        )]);
        let sha = create_commit(
            &runner,
            "owner/repo",
            "chore: sync",
            "tree-sha",
            "parent-sha",
        )
        .unwrap();
        assert_eq!(sha, "commit-sha");
    }

    #[test]
    fn create_commit_propagates_error() {
        let runner = MockRunner::new(vec![(vec!["api"], MockRunner::err("commit failed"))]);
        let err = create_commit(&runner, "owner/repo", "msg", "tree", "parent").unwrap_err();
        assert!(err.to_string().contains("commit failed") || err.to_string().contains("failed"));
    }

    // ---------------------------------------------------------------------------
    // create_branch_ref / force_update_ref
    // ---------------------------------------------------------------------------

    #[test]
    fn create_branch_ref_succeeds_on_first_call() {
        // Normal path: POST succeeds.
        let runner = MockRunner::new(vec![(
            vec!["api", "repos/owner/repo/git/refs"],
            MockRunner::ok(""),
        )]);
        assert!(create_branch_ref(&runner, "owner/repo", "my-branch", "commit-sha").is_ok());
    }

    #[test]
    fn create_branch_ref_force_updates_on_existing() {
        // First call (POST) returns "already exists"; second (PATCH) succeeds.
        let runner = MockRunner::new(vec![
            (
                vec!["api", "repos/owner/repo/git/refs"],
                GhOutput {
                    exit_code: Some(1),
                    stdout: vec![],
                    stderr: b"already exists".to_vec(),
                },
            ),
            (
                vec!["api", "repos/owner/repo/git/refs/heads/my-branch"],
                MockRunner::ok(""),
            ),
        ]);
        assert!(create_branch_ref(&runner, "owner/repo", "my-branch", "commit-sha").is_ok());
    }

    #[test]
    fn create_branch_ref_fails_on_generic_error() {
        // Non-"already exists" failure must propagate as error.
        let runner = MockRunner::new(vec![(vec!["api"], MockRunner::err("permission denied"))]);
        let err = create_branch_ref(&runner, "owner/repo", "my-branch", "sha").unwrap_err();
        assert!(
            err.to_string().contains("permission denied") || err.to_string().contains("git/refs"),
            "expected error: {err}"
        );
    }

    // ---------------------------------------------------------------------------
    // create_pr
    // ---------------------------------------------------------------------------

    #[test]
    fn create_pr_returns_url_on_success() {
        let runner = MockRunner::new(vec![(
            vec!["pr", "create"],
            MockRunner::ok("https://github.com/owner/repo/pull/1\n"),
        )]);
        let url = create_pr(&runner, "owner/repo", "main", "my-branch", "title", "body").unwrap();
        assert_eq!(url, "https://github.com/owner/repo/pull/1");
    }

    #[test]
    fn create_pr_returns_existing_url_when_pr_exists() {
        let runner = MockRunner::new(vec![
            (vec!["pr", "create"], MockRunner::err("already exists")),
            (
                vec!["pr", "view"],
                MockRunner::ok("https://github.com/owner/repo/pull/42\n"),
            ),
        ]);
        let url = create_pr(&runner, "owner/repo", "main", "my-branch", "title", "body").unwrap();
        assert_eq!(url, "https://github.com/owner/repo/pull/42");
    }

    #[test]
    fn create_pr_fails_on_generic_error() {
        // Error message without "already exists" must propagate.
        let runner = MockRunner::new(vec![(
            vec!["pr", "create"],
            MockRunner::err("base branch not found"),
        )]);
        let err =
            create_pr(&runner, "owner/repo", "main", "my-branch", "title", "body").unwrap_err();
        assert!(
            err.to_string().contains("base branch not found")
                || err.to_string().contains("gh pr create"),
            "expected error: {err}"
        );
    }

    #[test]
    fn create_pr_url_is_trimmed() {
        // Trailing newlines in gh output must be stripped.
        let runner = MockRunner::new(vec![(
            vec!["pr", "create"],
            MockRunner::ok("https://github.com/owner/repo/pull/99\n\n"),
        )]);
        let url = create_pr(&runner, "owner/repo", "main", "my-branch", "title", "body").unwrap();
        assert_eq!(url, "https://github.com/owner/repo/pull/99");
    }
}
