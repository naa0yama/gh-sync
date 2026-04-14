//! Human-readable preview output for spec changes.
// TODO: add per-item doc comments to satisfy `missing_docs` and `missing_errors_doc`
#![allow(missing_docs)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::must_use_candidate)]

use std::io::Write;
use std::process::ExitCode;

use gh_sync_manifest::{PullRequestRule, RefNameCondition, RequiredStatusChecks, Ruleset};

use super::SpecChange;

// ---------------------------------------------------------------------------
// Output helpers
// ---------------------------------------------------------------------------

/// Return a padded tag string with optional ANSI color.
///
/// Color is automatically suppressed when the output is not a TTY or when
/// `NO_COLOR` / `TERM=dumb` is set (`console::colors_enabled()` returns false).
fn styled_tag(label: &str) -> String {
    use console::style;
    let s = format!("[{label:<7}]");
    if !console::colors_enabled() {
        return s;
    }
    match label {
        "OK" => format!("{}", style(s).dim()),
        "CHANGED" | "ADD" => format!("{}", style(s).yellow()),
        "DELETE" => format!("{}", style(s).red()),
        _ => s,
    }
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

/// Print a human-readable preview of `changes` to `w`.
///
/// Returns `(exit_code, has_changes)` where `exit_code` is non-zero when
/// drift was detected.
///
/// # Errors
///
/// Returns an error if writing to `w` fails.
pub fn print_preview(
    w: &mut dyn Write,
    changes: &[SpecChange],
    repo: &str,
) -> std::io::Result<(ExitCode, bool)> {
    writeln!(w, "=== repo: {repo} ===")?;

    let mut changed_count = 0usize;
    let mut ok_count = 0usize;

    // First pass: repository settings, rulesets, and branch protections.
    for change in changes {
        print_non_label_change(w, change, &mut changed_count, &mut ok_count)?;
    }

    // Second pass: labels in a dedicated section at the bottom.
    print_labels_section(w, changes, &mut changed_count, &mut ok_count)?;

    writeln!(w, "---")?;
    let has_actions = changed_count > 0;
    if has_actions {
        writeln!(w, "{changed_count} changed, {ok_count} up to date")?;
    } else {
        writeln!(w, "all settings up to date")?;
    }

    Ok((ExitCode::SUCCESS, has_actions))
}

fn print_non_label_change(
    w: &mut dyn Write,
    change: &SpecChange,
    changed: &mut usize,
    ok: &mut usize,
) -> std::io::Result<()> {
    match change {
        SpecChange::FieldChanged { field, old, new } => {
            writeln!(w, "{}  {field}: {old:?} → {new:?}", styled_tag("CHANGED"))?;
            *changed = changed.saturating_add(1);
        }
        SpecChange::FieldOk { field, value } => {
            writeln!(w, "{}  {field}: {value}", styled_tag("OK"))?;
            *ok = ok.saturating_add(1);
        }
        SpecChange::RulesetAdd { name, spec } => {
            writeln!(w, "{}  rulesets/{name}", styled_tag("ADD"))?;
            print_ruleset_detail(w, spec, None)?;
            *changed = changed.saturating_add(1);
        }
        SpecChange::RulesetUpdate {
            id,
            name,
            spec,
            live,
        } => {
            writeln!(w, "{}  rulesets/{name} (id={id})", styled_tag("CHANGED"))?;
            print_ruleset_detail(w, spec, live.as_deref())?;
            *changed = changed.saturating_add(1);
        }
        SpecChange::RulesetOk { id, name } => {
            writeln!(w, "{}  rulesets/{name} (id={id})", styled_tag("OK"))?;
            *ok = ok.saturating_add(1);
        }
        SpecChange::RulesetDelete { id, name } => {
            writeln!(w, "{}  rulesets/{name} (id={id})", styled_tag("DELETE"))?;
            *changed = changed.saturating_add(1);
        }
        SpecChange::BranchProtectionAdd { spec } => {
            writeln!(
                w,
                "{}  branch_protection/{}",
                styled_tag("ADD"),
                spec.pattern
            )?;
            *changed = changed.saturating_add(1);
        }
        SpecChange::BranchProtectionUpdate { spec } => {
            writeln!(
                w,
                "{}  branch_protection/{}",
                styled_tag("CHANGED"),
                spec.pattern
            )?;
            *changed = changed.saturating_add(1);
        }
        SpecChange::BranchProtectionRemove { pattern } => {
            writeln!(w, "{}  branch_protection/{pattern}", styled_tag("DELETE"))?;
            *changed = changed.saturating_add(1);
        }
        SpecChange::BranchProtectionOk { pattern } => {
            writeln!(w, "{}  branch_protection/{pattern}", styled_tag("OK"))?;
            *ok = ok.saturating_add(1);
        }
        // Labels are handled separately.
        SpecChange::LabelAdd { .. }
        | SpecChange::LabelUpdate { .. }
        | SpecChange::LabelDelete { .. }
        | SpecChange::LabelOk { .. } => {}
    }
    Ok(())
}

fn print_labels_section(
    w: &mut dyn Write,
    all_changes: &[SpecChange],
    n_changed: &mut usize,
    n_ok: &mut usize,
) -> std::io::Result<()> {
    let has_labels = all_changes.iter().any(|c| {
        matches!(
            c,
            SpecChange::LabelAdd { .. }
                | SpecChange::LabelUpdate { .. }
                | SpecChange::LabelDelete { .. }
                | SpecChange::LabelOk { .. }
        )
    });
    if !has_labels {
        return Ok(());
    }
    writeln!(w, "--- labels ---")?;
    for item in all_changes {
        match item {
            SpecChange::LabelAdd {
                name,
                color,
                description,
            } => {
                let desc = description.as_deref().unwrap_or("");
                writeln!(w, "{}  {name} (#{color}) {desc}", styled_tag("ADD"))?;
                *n_changed = n_changed.saturating_add(1);
            }
            SpecChange::LabelUpdate {
                name,
                old_color,
                old_description,
                new_color,
                new_description,
            } => {
                writeln!(
                    w,
                    "{}  {name}: #{old_color}/{} → #{new_color}/{}",
                    styled_tag("CHANGED"),
                    old_description.as_deref().unwrap_or(""),
                    new_description.as_deref().unwrap_or(""),
                )?;
                *n_changed = n_changed.saturating_add(1);
            }
            SpecChange::LabelDelete { name } => {
                writeln!(w, "{}  {name}", styled_tag("DELETE"))?;
                *n_changed = n_changed.saturating_add(1);
            }
            SpecChange::LabelOk { name } => {
                writeln!(w, "{}  {name}", styled_tag("OK"))?;
                *n_ok = n_ok.saturating_add(1);
            }
            _ => {}
        }
    }
    Ok(())
}

/// Write indented detail lines for a ruleset spec entry.
///
/// Called under a `[CHANGED]` or `[ADD    ]` ruleset line to show what the
/// spec declares, so the user can see what will be applied without reading
/// the manifest file.
/// Print a single ruleset field with optional [OK]/[CHANGED] status tag.
///
/// - `matches = None`        → no status tag (e.g. for ADD where no live data exists)
/// - `matches = Some(true)`  → `[OK     ]` prefix
/// - `matches = Some(false)` → `[CHANGED]` prefix with `(was: <live_display>)` suffix
fn write_ruleset_field(
    w: &mut dyn Write,
    indent: &str,
    name: &str,
    spec_display: &str,
    matches: Option<bool>,
    live_display: Option<&str>,
) -> std::io::Result<()> {
    match matches {
        Some(true) => writeln!(w, "{}{}  {name}: {spec_display}", indent, styled_tag("OK")),
        Some(false) => {
            let was = live_display.unwrap_or("?");
            writeln!(
                w,
                "{}{}  {name}: {spec_display} (was: {was})",
                indent,
                styled_tag("CHANGED")
            )
        }
        None => writeln!(w, "{indent}{name}: {spec_display}"),
    }
}

/// Print `ref_name` conditions (include/exclude) with optional per-field status.
fn print_ruleset_conditions(
    w: &mut dyn Write,
    ref_name: &RefNameCondition,
    live: Option<&serde_json::Value>,
) -> std::io::Result<()> {
    let live_rn = live
        .and_then(|v| v.get("conditions"))
        .and_then(|c| c.get("ref_name"));
    for (key, spec_list) in [
        ("include", ref_name.include.as_ref()),
        ("exclude", ref_name.exclude.as_ref()),
    ] {
        let Some(list) = spec_list else { continue };
        let spec_d = format!("{list:?}");
        let live_vals: Option<Vec<String>> = live_rn
            .and_then(|rn| rn.get(key))
            .and_then(serde_json::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| serde_json::Value::as_str(v).map(str::to_owned))
                    .collect()
            });
        let matches = live.map(|_| {
            live_vals.as_ref().is_some_and(|lv| {
                let mut s: Vec<&str> = list.iter().map(String::as_str).collect();
                let mut l: Vec<&str> = lv.iter().map(String::as_str).collect();
                s.sort_unstable();
                l.sort_unstable();
                s == l
            })
        });
        let live_d = live_vals.map(|lv| format!("{lv:?}"));
        write_ruleset_field(
            w,
            "      ",
            &format!("conditions.ref_name.{key}"),
            &spec_d,
            matches,
            live_d.as_deref(),
        )?;
    }
    Ok(())
}

/// Print `pull_request` rule fields with optional per-field status.
///
/// `has_live` indicates live data was available at all (so tags should be shown).
/// `live_params` is the `parameters` object of the `pull_request` rule entry, if present.
fn print_ruleset_pr_rule(
    w: &mut dyn Write,
    pr: &PullRequestRule,
    has_live: bool,
    live_params: Option<&serde_json::Value>,
) -> std::io::Result<()> {
    writeln!(w, "      rules.pull_request:")?;

    // Helpers: compute (matches, live_display) for bool/u64 fields.
    let cmp_bool = |key: &str, sv: bool| -> (Option<bool>, Option<String>) {
        let lv = live_params
            .and_then(|p| p.get(key))
            .and_then(serde_json::Value::as_bool);
        (
            if has_live { Some(lv == Some(sv)) } else { None },
            lv.map(|b| b.to_string()),
        )
    };
    let cmp_u64 = |key: &str, sv: u64| -> (Option<bool>, Option<String>) {
        let lv = live_params
            .and_then(|p| p.get(key))
            .and_then(serde_json::Value::as_u64);
        (
            if has_live { Some(lv == Some(sv)) } else { None },
            lv.map(|n| n.to_string()),
        )
    };

    if let Some(v) = pr.required_approving_review_count {
        let (matches, live_d) = cmp_u64("required_approving_review_count", u64::from(v));
        write_ruleset_field(
            w,
            "        ",
            "required_approving_review_count",
            &v.to_string(),
            matches,
            live_d.as_deref(),
        )?;
    }
    for (key, val) in [
        (
            "dismiss_stale_reviews_on_push",
            pr.dismiss_stale_reviews_on_push,
        ),
        ("require_code_owner_review", pr.require_code_owner_review),
        ("require_last_push_approval", pr.require_last_push_approval),
        (
            "required_review_thread_resolution",
            pr.required_review_thread_resolution,
        ),
    ] {
        if let Some(v) = val {
            let (matches, live_d) = cmp_bool(key, v);
            write_ruleset_field(
                w,
                "        ",
                key,
                &v.to_string(),
                matches,
                live_d.as_deref(),
            )?;
        }
    }
    if let Some(methods) = &pr.allowed_merge_methods {
        let spec_d = format!("{methods:?}");
        let live_methods: Option<Vec<String>> = live_params
            .and_then(|p| p.get("allowed_merge_methods"))
            .and_then(serde_json::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| serde_json::Value::as_str(v).map(str::to_owned))
                    .collect()
            });
        let matches = if has_live {
            Some(live_methods.as_ref().is_some_and(|lm| {
                let mut s: Vec<&str> = methods.iter().map(String::as_str).collect();
                let mut l: Vec<&str> = lm.iter().map(String::as_str).collect();
                s.sort_unstable();
                l.sort_unstable();
                s == l
            }))
        } else {
            None
        };
        let live_d = live_methods.map(|lm| format!("{lm:?}"));
        write_ruleset_field(
            w,
            "        ",
            "allowed_merge_methods",
            &spec_d,
            matches,
            live_d.as_deref(),
        )?;
    }
    Ok(())
}

/// Print `required_status_checks` rule fields with optional per-field status.
fn print_ruleset_sc_rule(
    w: &mut dyn Write,
    sc: &RequiredStatusChecks,
    has_live: bool,
    live_params: Option<&serde_json::Value>,
) -> std::io::Result<()> {
    writeln!(w, "      rules.required_status_checks:")?;
    if let Some(v) = sc.strict_required_status_checks_policy {
        let lv = live_params
            .and_then(|p| p.get("strict_required_status_checks_policy"))
            .and_then(serde_json::Value::as_bool);
        let matches = if has_live { Some(lv == Some(v)) } else { None };
        let live_d = lv.map(|b| b.to_string());
        write_ruleset_field(
            w,
            "        ",
            "strict_required_status_checks_policy",
            &v.to_string(),
            matches,
            live_d.as_deref(),
        )?;
    }
    if let Some(ctxs) = &sc.contexts {
        let names: Vec<&str> = ctxs.iter().map(|c| c.context.as_str()).collect();
        let spec_d = format!("{names:?}");
        let live_ctx_names: Option<Vec<String>> = live_params
            .and_then(|p| p.get("required_status_checks"))
            .and_then(serde_json::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|c| {
                        c.get("context")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned)
                    })
                    .collect()
            });
        let matches = if has_live {
            Some(live_ctx_names.as_ref().is_some_and(|ln| {
                let mut s: Vec<&str> = ctxs.iter().map(|c| c.context.as_str()).collect();
                let mut l: Vec<&str> = ln.iter().map(String::as_str).collect();
                s.sort_unstable();
                l.sort_unstable();
                s == l
            }))
        } else {
            None
        };
        let live_d = live_ctx_names.map(|ln| format!("{ln:?}"));
        write_ruleset_field(
            w,
            "        ",
            "contexts",
            &spec_d,
            matches,
            live_d.as_deref(),
        )?;
    }
    Ok(())
}

fn print_ruleset_detail(
    w: &mut dyn Write,
    rs: &Ruleset,
    live: Option<&serde_json::Value>,
) -> std::io::Result<()> {
    let live_rules: &[serde_json::Value] = live
        .and_then(|v| v.get("rules"))
        .and_then(serde_json::Value::as_array)
        .map_or(&[], Vec::as_slice);

    if let Some(t) = &rs.target {
        let lv = live
            .and_then(|v| v.get("target"))
            .and_then(serde_json::Value::as_str);
        write_ruleset_field(w, "      ", "target", t, lv.map(|v| v == t), lv)?;
    }
    if let Some(e) = &rs.enforcement {
        let lv = live
            .and_then(|v| v.get("enforcement"))
            .and_then(serde_json::Value::as_str);
        write_ruleset_field(w, "      ", "enforcement", e, lv.map(|v| v == e), lv)?;
    }

    if let Some(cond) = &rs.conditions
        && let Some(ref_name) = &cond.ref_name
    {
        print_ruleset_conditions(w, ref_name, live)?;
    }

    if let Some(rules) = &rs.rules {
        let live_rule_types: std::collections::HashSet<&str> = live_rules
            .iter()
            .filter_map(|r| r.get("type").and_then(serde_json::Value::as_str))
            .collect();
        let live_pr: Option<&serde_json::Value> = live_rules
            .iter()
            .find(|r| r.get("type").and_then(serde_json::Value::as_str) == Some("pull_request"))
            .and_then(|r| r.get("parameters"));
        let live_sc: Option<&serde_json::Value> = live_rules
            .iter()
            .find(|r| {
                r.get("type").and_then(serde_json::Value::as_str) == Some("required_status_checks")
            })
            .and_then(|r| r.get("parameters"));
        let has_live = live.is_some();

        for (flag_name, spec_val) in [
            ("non_fast_forward", rules.non_fast_forward),
            ("deletion", rules.deletion),
            ("creation", rules.creation),
            ("required_linear_history", rules.required_linear_history),
            ("required_signatures", rules.required_signatures),
        ] {
            if let Some(v) = spec_val {
                let spec_d = if v { "true" } else { "false" };
                let live_present = live_rule_types.contains(flag_name);
                let matches = if has_live {
                    Some(live_present == v)
                } else {
                    None
                };
                let live_d: Option<&str> = if has_live {
                    Some(if live_present { "true" } else { "false" })
                } else {
                    None
                };
                write_ruleset_field(
                    w,
                    "      ",
                    &format!("rules.{flag_name}"),
                    spec_d,
                    matches,
                    live_d,
                )?;
            }
        }

        if let Some(pr) = &rules.pull_request {
            print_ruleset_pr_rule(w, pr, has_live, live_pr)?;
        }
        if let Some(sc) = &rules.required_status_checks {
            print_ruleset_sc_rule(w, sc, has_live, live_sc)?;
        }
    }

    Ok(())
}
