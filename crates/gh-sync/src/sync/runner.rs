use super::gh_error;

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

        // When the tracing level is DEBUG or lower, propagate GH_DEBUG=api so
        // the gh CLI emits its HTTP request/response trace to stderr.
        if tracing::enabled!(tracing::Level::DEBUG) {
            cmd.env("GH_DEBUG", "api");
        }

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
// run_checked — structured error helper
// ---------------------------------------------------------------------------

/// Run `gh <args>` and return the output, or bail with a rich error on
/// non-zero exit.
///
/// On failure this function:
/// 1. Parses API error JSON from stdout/stderr via [`gh_error::parse_from_streams`].
/// 2. Emits a structured `tracing::error!` with the full stdout/stderr bodies.
/// 3. Bails with a human-readable message containing `op`, a tail-truncated
///    stderr (≤ 2 KiB), and any parsed API-error fields.
///
/// `op` should be a short human-readable label such as `"PATCH repos/owner/repo"`.
pub fn run_checked(
    runner: &dyn GhRunner,
    args: &[&str],
    stdin: Option<&[u8]>,
    op: &str,
) -> anyhow::Result<GhOutput> {
    if let Some(bytes) = stdin {
        let body_len = bytes.len();
        let preview_len = body_len.min(1024);
        let body_preview = bytes
            .get(..preview_len)
            .and_then(|b| std::str::from_utf8(b).ok())
            .map_or_else(|| "<binary>".to_owned(), str::to_owned);
        tracing::debug!(%op, ?args, body_len, body_preview, "gh command stdin");
    }

    let out = runner.run(args, stdin)?;

    if out.success() {
        return Ok(out);
    }

    let exit_code = out.exit_code;
    let stderr_full = String::from_utf8_lossy(&out.stderr);
    let stdout_full = String::from_utf8_lossy(&out.stdout);
    let api_error = gh_error::parse_from_streams(&out.stdout, &out.stderr);

    tracing::error!(
        %op,
        ?args,
        ?exit_code,
        stderr = %stderr_full,
        stdout = %stdout_full,
        ?api_error,
        "gh command failed"
    );

    let stderr_summary = gh_error::truncate_tail(&stderr_full, 2048);
    let detail = build_error_detail(api_error.as_ref(), &stdout_full);

    anyhow::bail!("{op} failed (exit {exit_code:?}): {stderr_summary}{detail}");
}

fn build_error_detail(api_error: Option<&gh_error::GhApiError>, stdout: &str) -> String {
    api_error.map_or_else(
        || {
            if stdout.is_empty() {
                String::new()
            } else {
                let stdout_summary = gh_error::truncate_tail(stdout, 2048);
                format!(" [stdout: {stdout_summary}]")
            }
        },
        |err| {
            let mut parts: Vec<String> = Vec::new();
            if let Some(msg) = &err.message {
                parts.push(format!("message: {msg}"));
            }
            if let Some(status) = err.status {
                parts.push(format!("HTTP {status}"));
            }
            if let Some(rid) = &err.request_id {
                parts.push(format!("request_id={rid}"));
            }
            for fe in &err.errors {
                let mut fparts: Vec<String> = Vec::new();
                if let Some(r) = &fe.resource {
                    fparts.push(format!("resource={r}"));
                }
                if let Some(f) = &fe.field {
                    fparts.push(format!("field={f}"));
                }
                if let Some(c) = &fe.code {
                    fparts.push(format!("code={c}"));
                }
                if let Some(m) = &fe.message {
                    fparts.push(format!("msg={m}"));
                }
                if !fparts.is_empty() {
                    parts.push(fparts.join(", "));
                }
            }
            if parts.is_empty() {
                String::new()
            } else {
                format!(" [{}]", parts.join("; "))
            }
        },
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal mock runner for run_checked tests.
    struct MockRunner {
        output: GhOutput,
    }

    impl GhRunner for MockRunner {
        fn run(&self, _args: &[&str], _stdin: Option<&[u8]>) -> anyhow::Result<GhOutput> {
            Ok(self.output.clone())
        }
    }

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

    #[test]
    fn run_checked_returns_ok_on_success() {
        let runner = MockRunner {
            output: GhOutput {
                exit_code: Some(0),
                stdout: b"ok".to_vec(),
                stderr: vec![],
            },
        };
        let result = run_checked(&runner, &["api", "repos/foo"], None, "GET repos/foo");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().stdout, b"ok");
    }

    #[test]
    fn run_checked_error_includes_stderr_in_message() {
        let runner = MockRunner {
            output: GhOutput {
                exit_code: Some(1),
                stdout: vec![],
                stderr: b"something went wrong".to_vec(),
            },
        };
        let err = run_checked(&runner, &["api"], None, "GET foo")
            .unwrap_err()
            .to_string();
        assert!(err.contains("something went wrong"), "err={err}");
    }

    #[test]
    fn run_checked_error_includes_api_error_fields_in_message() {
        let runner = MockRunner {
            output: GhOutput {
                exit_code: Some(1),
                stdout: br#"{"message":"Validation Failed","errors":[{"resource":"Repository","field":"merge_commit_title","code":"invalid"}],"status":"422"}"#.to_vec(),
                stderr: b"gh: Validation Failed (HTTP 422)".to_vec(),
            },
        };
        let err = run_checked(
            &runner,
            &["api", "-X", "PATCH", "repos/foo"],
            None,
            "PATCH repos/foo",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("Validation Failed"), "err={err}");
        assert!(err.contains("merge_commit_title"), "err={err}");
        assert!(err.contains("HTTP 422"), "err={err}");
    }

    #[test]
    fn run_checked_truncates_large_stderr_in_bail() {
        // 10 KiB stderr — bail message must be well under 3 KiB.
        let large_stderr = vec![b'x'; 10 * 1024];
        let runner = MockRunner {
            output: GhOutput {
                exit_code: Some(1),
                stdout: vec![],
                stderr: large_stderr,
            },
        };
        let err = run_checked(&runner, &["api"], None, "PATCH foo")
            .unwrap_err()
            .to_string();
        assert!(
            err.len() < 3 * 1024,
            "bail message too long: {} bytes",
            err.len()
        );
        assert!(
            err.contains("truncated"),
            "should contain truncation notice; err={err}"
        );
    }

    #[test]
    fn run_checked_includes_stdout_when_no_api_error_parsed() {
        let runner = MockRunner {
            output: GhOutput {
                exit_code: Some(1),
                stdout: b"unexpected plain output".to_vec(),
                stderr: b"exit 1".to_vec(),
            },
        };
        let err = run_checked(&runner, &["api"], None, "GET bar")
            .unwrap_err()
            .to_string();
        assert!(err.contains("unexpected plain output"), "err={err}");
    }
}
