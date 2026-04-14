//! Formatted output for the `sync` subcommand.
//!
//! All public functions accept a `&mut dyn Write` so callers can redirect
//! output to a `Vec<u8>` in tests or to stdout in production.

use std::io::Write;

use gh_sync_manifest::{Rule, Strategy};

use crate::strategy::StrategyResult;

// ---------------------------------------------------------------------------
// RuleOutcome
// ---------------------------------------------------------------------------

/// Paired result of applying one sync rule.
#[allow(missing_debug_implementations)] // Rule has Debug but StrategyResult::Changed content is bytes
#[allow(dead_code)] // fields are set for completeness; only `result` is consumed today
pub struct RuleOutcome<'a> {
    /// The rule that was evaluated.
    pub rule: &'a Rule,
    /// The outcome of applying the rule's strategy.
    pub result: StrategyResult,
    /// Unified diff text (non-empty only when relevant to the mode).
    pub diff: String,
}

// ---------------------------------------------------------------------------
// StatusTag
// ---------------------------------------------------------------------------

/// Human-readable status tag printed at the start of each rule line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusTag {
    /// File content changed (or would change in dry-run / ci-check).
    Changed,
    /// File is already in the expected state.
    Ok,
    /// Rule was intentionally skipped.
    Skipped,
    /// File was deleted (or would be deleted).
    Deleted,
    /// Detected drift between local and expected state (`--ci-check`).
    Drift,
    /// Patch application conflicted.
    Warn,
    /// Rule application failed.
    Fail,
}

impl StatusTag {
    const fn label(self) -> &'static str {
        match self {
            Self::Changed => "CHANGED",
            Self::Ok => "OK",
            Self::Skipped => "SKIPPED",
            Self::Deleted => "DELETED",
            Self::Drift => "DRIFT",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
        }
    }

    /// Return the padded tag string with ANSI color when the output is a TTY.
    ///
    /// Color is suppressed automatically when `NO_COLOR` / `TERM=dumb` is set
    /// or when the writer is not a TTY (`console::colors_enabled()` returns false).
    pub(crate) fn styled(self) -> String {
        use console::style;
        let s = format!("[{:<7}]", self.label());
        if !console::colors_enabled() {
            return s;
        }
        match self {
            Self::Ok | Self::Skipped => format!("{}", style(s).dim()),
            Self::Changed | Self::Drift | Self::Warn => format!("{}", style(s).yellow()),
            Self::Deleted | Self::Fail => format!("{}", style(s).red().bold()),
        }
    }
}

// ---------------------------------------------------------------------------
// emit_status
// ---------------------------------------------------------------------------

/// Write a single `[TAG]  path (strategy)[: detail]` line.
///
/// # Errors
/// Propagates `Write` errors.
pub fn emit_status(
    w: &mut dyn Write,
    tag: StatusTag,
    path: &str,
    strategy: Strategy,
    detail: Option<&str>,
) -> std::io::Result<()> {
    let styled = tag.styled();
    match detail {
        Some(d) => writeln!(w, "{styled}  {path} ({strategy}): {d}"),
        None => writeln!(w, "{styled}  {path} ({strategy})"),
    }
}

/// Write a diff block with ANSI syntax highlighting, followed by a blank line.
///
/// Does nothing when `diff` is empty.
/// Colors are suppressed automatically when the output is not a TTY or when
/// `NO_COLOR` / `TERM=dumb` is set.
///
/// # Errors
/// Propagates `Write` errors.
pub fn emit_diff(w: &mut dyn Write, diff: &str) -> std::io::Result<()> {
    if diff.is_empty() {
        return Ok(());
    }
    let colored = colorize_diff(diff);
    write!(w, "{colored}")?;
    writeln!(w)
}

/// Apply ANSI color codes to a unified diff string.
///
/// Returns the original string unchanged when colors are disabled (non-TTY,
/// `NO_COLOR`, or `TERM=dumb`).
#[must_use]
pub fn colorize_diff(diff: &str) -> String {
    use console::style;

    if !console::colors_enabled() {
        return diff.to_owned();
    }

    diff.lines()
        .map(|line| {
            if line.starts_with("---") || line.starts_with("+++") {
                // File header lines: bold
                format!("{}\n", style(line).bold())
            } else if line.starts_with("@@") {
                // Hunk header: cyan
                format!("{}\n", style(line).cyan())
            } else if line.starts_with('+') {
                // Addition: green
                format!("{}\n", style(line).green())
            } else if line.starts_with('-') {
                // Deletion: red
                format!("{}\n", style(line).red())
            } else {
                format!("{line}\n")
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// emit_summary
// ---------------------------------------------------------------------------

/// Summary counters collected across all outcomes.
#[derive(Debug, Default)]
pub struct Summary {
    /// Rules that produced a file change.
    pub changed: usize,
    /// Rules that left the file unchanged.
    pub unchanged: usize,
    /// Rules that were intentionally skipped.
    pub skipped: usize,
    /// Rules that deleted a file.
    pub deleted: usize,
    /// Rules that conflicted (patch strategy).
    pub conflicts: usize,
    /// Rules that returned an error.
    pub errors: usize,
}

impl Summary {
    /// Build a `Summary` by counting outcomes in `outcomes`.
    #[must_use]
    pub fn from_outcomes(outcomes: &[RuleOutcome<'_>]) -> Self {
        let mut s = Self::default();
        for o in outcomes {
            match &o.result {
                StrategyResult::Changed { .. } => s.changed = s.changed.saturating_add(1),
                StrategyResult::Unchanged => s.unchanged = s.unchanged.saturating_add(1),
                StrategyResult::Skipped { .. } => s.skipped = s.skipped.saturating_add(1),
                StrategyResult::Deleted => s.deleted = s.deleted.saturating_add(1),
                StrategyResult::Conflict { .. } => s.conflicts = s.conflicts.saturating_add(1),
                StrategyResult::Error(_) => s.errors = s.errors.saturating_add(1),
            }
        }
        s
    }

    /// Build a `Summary` for ci-check drift outcomes.
    ///
    /// `drift` counts outcomes where the local state differs from expected.
    #[must_use]
    pub fn from_drift_outcomes(outcomes: &[DriftOutcome<'_>]) -> DriftSummary {
        let mut drifted = 0usize;
        let mut up_to_date = 0usize;
        for o in outcomes {
            if o.drifted {
                drifted = drifted.saturating_add(1);
            } else {
                up_to_date = up_to_date.saturating_add(1);
            }
        }
        DriftSummary {
            drifted,
            up_to_date,
        }
    }
}

/// Write the summary footer line (e.g. `2 changed, 1 up to date`).
///
/// # Errors
/// Propagates `Write` errors.
pub fn emit_summary(w: &mut dyn Write, summary: &Summary) -> std::io::Result<()> {
    writeln!(w, "---")?;

    let mut parts: Vec<String> = Vec::new();
    if summary.changed > 0 {
        parts.push(format!("{} changed", summary.changed));
    }
    if summary.deleted > 0 {
        parts.push(format!("{} deleted", summary.deleted));
    }
    if summary.unchanged > 0 {
        parts.push(format!("{} up to date", summary.unchanged));
    }
    if summary.skipped > 0 {
        parts.push(format!("{} skipped", summary.skipped));
    }
    if summary.conflicts > 0 {
        parts.push(format!("{} conflict(s)", summary.conflicts));
    }
    if summary.errors > 0 {
        parts.push(format!("{} error(s)", summary.errors));
    }

    if parts.is_empty() {
        writeln!(w, "no files processed")
    } else {
        writeln!(w, "{}", parts.join(", "))
    }
}

// ---------------------------------------------------------------------------
// Drift output (--ci-check)
// ---------------------------------------------------------------------------

/// A single rule evaluation result for `--ci-check` mode.
#[allow(missing_debug_implementations)] // Rule reference — Debug on the wrapper is not needed
#[allow(dead_code)] // `detail` is set for completeness; `rule`/`drifted`/`diff` are consumed
pub struct DriftOutcome<'a> {
    /// The evaluated rule.
    pub rule: &'a Rule,
    /// `true` when local state diverges from the expected state.
    pub drifted: bool,
    /// Human-readable detail (e.g. "upstream has changes").
    pub detail: String,
    /// Unified diff of local vs expected (empty when `drifted = false`).
    pub diff: String,
}

/// Summary for drift detection.
#[derive(Debug, Default)]
pub struct DriftSummary {
    /// Number of files that drifted.
    pub drifted: usize,
    /// Number of files that are up to date.
    pub up_to_date: usize,
}

/// Write the drift summary footer.
///
/// # Errors
/// Propagates `Write` errors.
pub fn emit_drift_summary(w: &mut dyn Write, summary: &DriftSummary) -> std::io::Result<()> {
    writeln!(w, "---")?;
    if summary.drifted == 0 {
        writeln!(w, "all files up to date")
    } else {
        writeln!(
            w,
            "{} file(s) drifted, {} up to date",
            summary.drifted, summary.up_to_date
        )
    }
}

// ---------------------------------------------------------------------------
// GHA annotations
// ---------------------------------------------------------------------------

/// Emit GitHub Actions workflow command annotations for each drifted file.
///
/// Output goes to `w` (typically stdout). GHA reads these commands from stdout.
///
/// # Errors
/// Propagates `Write` errors.
pub fn emit_gha_annotations(
    w: &mut dyn Write,
    outcomes: &[DriftOutcome<'_>],
) -> std::io::Result<()> {
    for o in outcomes {
        if o.drifted {
            writeln!(
                w,
                "::error file={path},title=gh-sync drift::{path} is out of sync with upstream",
                path = o.rule.path
            )?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// PR comment
// ---------------------------------------------------------------------------

/// Build the PR comment body for drifted files.
///
/// Returns `None` when no files drifted.
#[must_use]
pub fn build_pr_comment(outcomes: &[DriftOutcome<'_>]) -> Option<String> {
    use std::fmt::Write as _;

    let drifted: Vec<&DriftOutcome<'_>> = outcomes.iter().filter(|o| o.drifted).collect();
    if drifted.is_empty() {
        return None;
    }

    let mut body = String::from("## gh-sync drift detected\n\n");
    body.push_str("| file | strategy | status |\n");
    body.push_str("| ---- | -------- | ------ |\n");
    for o in &drifted {
        let _ = writeln!(body, "| `{}` | {} | DRIFT |", o.rule.path, o.rule.strategy);
    }

    for o in &drifted {
        if !o.diff.is_empty() {
            let _ = write!(
                body,
                "\n<details>\n<summary>{} diff</summary>\n\n```diff\n{}```\n\n</details>\n",
                o.rule.path, o.diff
            );
        }
    }

    body.push_str("\nRun `gh-sync sync` locally to apply upstream changes.\n");
    Some(body)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::panic)]

    use gh_sync_manifest::{Rule, Strategy};

    use super::*;

    fn rule(path: &str, strategy: Strategy) -> Rule {
        Rule {
            path: path.to_owned(),
            strategy,
            source: None,
            patch: None,
        }
    }

    // ------------------------------------------------------------------
    // emit_status
    // ------------------------------------------------------------------

    #[test]
    fn emit_status_changed_no_detail() {
        // Arrange
        let r = rule(".github/ci.yml", Strategy::Replace);
        let mut buf: Vec<u8> = Vec::new();

        // Act
        emit_status(&mut buf, StatusTag::Changed, &r.path, r.strategy, None).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("[CHANGED]"), "missing tag: {out}");
        assert!(out.contains(".github/ci.yml"), "missing path: {out}");
        assert!(out.contains("replace"), "missing strategy: {out}");
    }

    #[test]
    fn emit_status_skipped_with_detail() {
        // Arrange
        let r = rule("file.txt", Strategy::CreateOnly);
        let mut buf: Vec<u8> = Vec::new();

        // Act
        emit_status(
            &mut buf,
            StatusTag::Skipped,
            &r.path,
            r.strategy,
            Some("already exists"),
        )
        .unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("[SKIPPED]"), "missing tag");
        assert!(out.contains("already exists"), "missing detail: {out}");
    }

    #[test]
    fn emit_status_all_tags() {
        // Verify every tag label renders correctly
        let cases = [
            (StatusTag::Changed, "CHANGED"),
            (StatusTag::Ok, "OK"),
            (StatusTag::Skipped, "SKIPPED"),
            (StatusTag::Deleted, "DELETED"),
            (StatusTag::Drift, "DRIFT"),
            (StatusTag::Warn, "WARN"),
            (StatusTag::Fail, "FAIL"),
        ];
        for (tag, expected_label) in cases {
            let mut buf: Vec<u8> = Vec::new();
            emit_status(&mut buf, tag, "x", Strategy::Replace, None).unwrap();
            let out = String::from_utf8(buf).unwrap();
            assert!(
                out.contains(expected_label),
                "tag {expected_label} not found in: {out}"
            );
        }
    }

    // ------------------------------------------------------------------
    // emit_diff
    // ------------------------------------------------------------------

    #[test]
    fn emit_diff_empty_writes_nothing() {
        // Arrange
        let mut buf: Vec<u8> = Vec::new();

        // Act
        emit_diff(&mut buf, "").unwrap();

        // Assert
        assert!(buf.is_empty(), "expected no output for empty diff");
    }

    #[test]
    fn emit_diff_non_empty_writes_diff_and_blank_line() {
        // Arrange
        let diff = "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n";
        let mut buf: Vec<u8> = Vec::new();

        // Act
        emit_diff(&mut buf, diff).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("--- a/x"), "missing diff header: {out}");
        // trailing blank line
        assert!(
            out.ends_with("\n\n"),
            "expected trailing blank line: {out:?}"
        );
    }

    // ------------------------------------------------------------------
    // emit_summary
    // ------------------------------------------------------------------

    #[test]
    fn summary_counts_outcomes() {
        // Arrange
        let r = rule("a", Strategy::Replace);
        let outcomes = vec![
            RuleOutcome {
                rule: &r,
                result: StrategyResult::Changed {
                    content: b"x".to_vec(),
                },
                diff: String::new(),
            },
            RuleOutcome {
                rule: &r,
                result: StrategyResult::Unchanged,
                diff: String::new(),
            },
            RuleOutcome {
                rule: &r,
                result: StrategyResult::Skipped {
                    reason: String::from("r"),
                },
                diff: String::new(),
            },
        ];

        // Act
        let summary = Summary::from_outcomes(&outcomes);

        // Assert
        assert_eq!(summary.changed, 1);
        assert_eq!(summary.unchanged, 1);
        assert_eq!(summary.skipped, 1);
    }

    #[test]
    fn emit_summary_formats_parts() {
        // Arrange
        let summary = Summary {
            changed: 2,
            unchanged: 1,
            deleted: 1,
            ..Summary::default()
        };
        let mut buf: Vec<u8> = Vec::new();

        // Act
        emit_summary(&mut buf, &summary).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("2 changed"), "missing changed: {out}");
        assert!(out.contains("1 deleted"), "missing deleted: {out}");
        assert!(out.contains("1 up to date"), "missing unchanged: {out}");
    }

    #[test]
    fn emit_summary_no_rules() {
        // Arrange
        let summary = Summary::default();
        let mut buf: Vec<u8> = Vec::new();

        // Act
        emit_summary(&mut buf, &summary).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("no files processed"), "unexpected: {out}");
    }

    // ------------------------------------------------------------------
    // emit_drift_summary
    // ------------------------------------------------------------------

    #[test]
    fn drift_summary_all_up_to_date() {
        let summary = DriftSummary {
            drifted: 0,
            up_to_date: 3,
        };
        let mut buf: Vec<u8> = Vec::new();
        emit_drift_summary(&mut buf, &summary).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("all files up to date"), "unexpected: {out}");
    }

    #[test]
    fn drift_summary_some_drifted() {
        let summary = DriftSummary {
            drifted: 2,
            up_to_date: 1,
        };
        let mut buf: Vec<u8> = Vec::new();
        emit_drift_summary(&mut buf, &summary).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("2 file(s) drifted"), "unexpected: {out}");
        assert!(out.contains("1 up to date"), "unexpected: {out}");
    }

    // ------------------------------------------------------------------
    // emit_gha_annotations
    // ------------------------------------------------------------------

    #[test]
    fn gha_annotations_only_for_drifted() {
        // Arrange
        let r1 = rule("a.yml", Strategy::Replace);
        let r2 = rule("b.toml", Strategy::Patch);
        let outcomes = vec![
            DriftOutcome {
                rule: &r1,
                drifted: true,
                detail: String::new(),
                diff: String::new(),
            },
            DriftOutcome {
                rule: &r2,
                drifted: false,
                detail: String::new(),
                diff: String::new(),
            },
        ];
        let mut buf: Vec<u8> = Vec::new();

        // Act
        emit_gha_annotations(&mut buf, &outcomes).unwrap();

        // Assert
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("::error"), "missing annotation: {out}");
        assert!(out.contains("a.yml"), "missing drifted file: {out}");
        assert!(
            !out.contains("b.toml"),
            "clean file should not appear: {out}"
        );
    }

    #[test]
    fn gha_annotations_empty_when_no_drift() {
        let outcomes: Vec<DriftOutcome<'_>> = vec![];
        let mut buf: Vec<u8> = Vec::new();
        emit_gha_annotations(&mut buf, &outcomes).unwrap();
        assert!(buf.is_empty(), "expected no output when no drift");
    }

    // ------------------------------------------------------------------
    // build_pr_comment
    // ------------------------------------------------------------------

    #[test]
    fn pr_comment_none_when_no_drift() {
        let r = rule("x", Strategy::Replace);
        let outcomes = vec![DriftOutcome {
            rule: &r,
            drifted: false,
            detail: String::new(),
            diff: String::new(),
        }];
        assert!(build_pr_comment(&outcomes).is_none());
    }

    #[test]
    fn pr_comment_contains_table_and_diff() {
        // Arrange
        let r = rule("ci.yml", Strategy::Replace);
        let outcomes = vec![DriftOutcome {
            rule: &r,
            drifted: true,
            detail: String::from("upstream has changes"),
            diff: String::from("@@ -1 +1 @@\n-old\n+new\n"),
        }];

        // Act
        let body = build_pr_comment(&outcomes).unwrap();

        // Assert
        assert!(
            body.contains("gh-sync drift detected"),
            "missing header: {body}"
        );
        assert!(body.contains("ci.yml"), "missing path: {body}");
        assert!(body.contains("replace"), "missing strategy: {body}");
        assert!(body.contains("@@ -1 +1 @@"), "missing diff: {body}");
    }

    // ------------------------------------------------------------------
    // Summary::from_drift_outcomes
    // ------------------------------------------------------------------

    #[test]
    fn from_drift_outcomes_counts_correctly() {
        // Arrange
        let r = rule("a", Strategy::Replace);
        let outcomes = vec![
            DriftOutcome {
                rule: &r,
                drifted: true,
                detail: String::new(),
                diff: String::new(),
            },
            DriftOutcome {
                rule: &r,
                drifted: false,
                detail: String::new(),
                diff: String::new(),
            },
            DriftOutcome {
                rule: &r,
                drifted: true,
                detail: String::new(),
                diff: String::new(),
            },
        ];

        // Act
        let s = Summary::from_drift_outcomes(&outcomes);

        // Assert
        assert_eq!(s.drifted, 2);
        assert_eq!(s.up_to_date, 1);
    }
}
