/// Raw output from a `gh` CLI invocation.
#[derive(Debug, Clone)]
pub struct GhOutput {
    /// Exit code returned by the process (`None` if the process was killed by signal).
    pub exit_code: Option<i32>,
    /// Bytes written to stdout.
    pub stdout: Vec<u8>,
    /// Bytes written to stderr.
    pub stderr: Vec<u8>,
}

impl GhOutput {
    /// Returns `true` when the process exited with code 0.
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }
}

// ---------------------------------------------------------------------------
// GhRunner trait
// ---------------------------------------------------------------------------

/// Abstracts spawning the `gh` CLI, enabling mock injection in tests.
///
/// Every `gh` invocation — whether `gh api …`, `gh label …`, or any other
/// subcommand — routes through this trait so that unit tests can inject a
/// [`MockGhRunner`] instead of spawning a real process.
#[allow(clippy::module_name_repetitions)]
pub trait GhRunner: Send + Sync {
    /// Run `gh <args>` and return the raw output.
    ///
    /// If `stdin` is `Some(bytes)`, the bytes are written to the process's
    /// stdin before waiting for it to finish.
    ///
    /// # Errors
    ///
    /// Returns an error if the process cannot be spawned (e.g. `gh` is not
    /// installed or not on `PATH`). A non-zero exit code is **not** an error
    /// at this level — callers inspect [`GhOutput::exit_code`] themselves.
    fn run(&self, args: &[&str], stdin: Option<&[u8]>) -> anyhow::Result<GhOutput>;
}

// ---------------------------------------------------------------------------
// SystemGhRunner — production implementation
// ---------------------------------------------------------------------------

/// Production [`GhRunner`] that spawns the real `gh` CLI.
#[allow(clippy::module_name_repetitions)] // "SystemGhRunner" in module "runner" is intentional
pub struct SystemGhRunner;

impl GhRunner for SystemGhRunner {
    #[cfg_attr(coverage_nightly, coverage(off))]
    fn run(&self, args: &[&str], stdin: Option<&[u8]>) -> anyhow::Result<GhOutput> {
        use anyhow::Context as _;
        use std::io::Write as _;

        let mut cmd = std::process::Command::new("gh");
        cmd.args(args);

        if stdin.is_some() {
            cmd.stdin(std::process::Stdio::piped());
        }
        // Always capture stdout and stderr so callers can inspect them.
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().context("failed to spawn `gh`")?;

        if let Some(bytes) = stdin
            && let Some(mut pipe) = child.stdin.take()
        {
            pipe.write_all(bytes)
                .context("failed to write stdin to `gh`")?;
        }

        let output = child
            .wait_with_output()
            .context("failed to wait for `gh`")?;

        Ok(GhOutput {
            exit_code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gh_output_success_true_on_zero() {
        let out = GhOutput {
            exit_code: Some(0),
            stdout: vec![],
            stderr: vec![],
        };
        assert!(out.success());
    }

    #[test]
    fn gh_output_success_false_on_nonzero() {
        let out = GhOutput {
            exit_code: Some(1),
            stdout: vec![],
            stderr: vec![],
        };
        assert!(!out.success());
    }

    #[test]
    fn gh_output_success_false_on_none() {
        let out = GhOutput {
            exit_code: None,
            stdout: vec![],
            stderr: vec![],
        };
        assert!(!out.success());
    }
}
