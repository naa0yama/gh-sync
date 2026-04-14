/// Interactive file + strategy picker widget.
///
/// Renders a scrollable list of files. The user can:
/// - Navigate with ↑/↓
/// - Toggle a file's inclusion with Space
/// - Cycle the strategy for the highlighted file with Tab
/// - Confirm with Enter (Escape / q cancels)
use std::io;

use anyhow::Context as _;
use dialoguer::console::{Key, Term, style};

use crate::sync::manifest::Strategy;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Number of file rows visible at once.
const PAGE_SIZE: usize = 20;

/// All strategies in cycle order.
const STRATEGIES: &[Strategy] = &[
    Strategy::Replace,
    Strategy::CreateOnly,
    Strategy::Delete,
    Strategy::Patch,
];

/// Width of the strategy column (length of the longest strategy name).
const STRATEGY_WIDTH: usize = 11; // "create_only"

/// Return the next index in a cycle of length `len`, wrapping 0 after the last.
#[must_use]
const fn cycle_next(idx: usize, len: usize) -> usize {
    let next = idx.saturating_add(1);
    if next >= len { 0 } else { next }
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A file selected by the user together with its assigned strategy.
#[derive(Debug)]
pub struct SelectedFile {
    /// Repository-relative file path.
    pub path: String,
    /// Synchronisation strategy chosen for this file.
    pub strategy: Strategy,
}

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

struct Item {
    path: String,
    strategy: Strategy,
    included: bool,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Show the interactive picker and return the files the user confirmed.
///
/// # Errors
///
/// Returns an error when terminal I/O fails, the user cancels (Escape / q),
/// or the user confirms without selecting any file.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn pick(paths: &[String]) -> anyhow::Result<Vec<SelectedFile>> {
    let term = Term::stderr();

    let mut items: Vec<Item> = paths
        .iter()
        .map(|p| Item {
            path: p.clone(),
            strategy: Strategy::Replace,
            included: false,
        })
        .collect();

    // `page` is constant once computed: the number of rows rendered each frame.
    let page = PAGE_SIZE.min(items.len());
    let mut cursor = 0usize;
    let mut offset = 0usize;

    term.write_line(
        "Select files and set strategy  \
         (↑↓ navigate, Space toggle, Tab cycle strategy, Enter confirm):",
    )
    .context("terminal write failed")?;

    render(&term, &items, cursor, offset, page)?;

    loop {
        match term.read_key().context("terminal read failed")? {
            Key::ArrowUp => {
                if cursor > 0 {
                    cursor = cursor.saturating_sub(1);
                    if cursor < offset {
                        offset = cursor;
                    }
                    redraw(&term, &items, cursor, offset, page)?;
                }
            }
            Key::ArrowDown => {
                let next = cursor.saturating_add(1);
                if next < items.len() {
                    cursor = next;
                    let viewport_end = offset.saturating_add(page);
                    if cursor >= viewport_end {
                        offset = cursor.saturating_add(1).saturating_sub(page);
                    }
                    redraw(&term, &items, cursor, offset, page)?;
                }
            }
            Key::Tab => {
                // Read current strategy without a mutable borrow, then update.
                let current = items.get(cursor).map(|i| i.strategy);
                if let Some(strategy) = current {
                    let idx = STRATEGIES.iter().position(|&s| s == strategy).unwrap_or(0);
                    let next_idx = cycle_next(idx, STRATEGIES.len());
                    if let (Some(item), Some(&next_strategy)) =
                        (items.get_mut(cursor), STRATEGIES.get(next_idx))
                    {
                        item.strategy = next_strategy;
                    }
                    redraw(&term, &items, cursor, offset, page)?;
                }
            }
            Key::Char(' ') => {
                if let Some(item) = items.get_mut(cursor) {
                    item.included = !item.included;
                }
                redraw(&term, &items, cursor, offset, page)?;
            }
            Key::Enter => break,
            Key::Escape | Key::Char('q') => {
                erase(&term, page)?;
                anyhow::bail!("selection cancelled");
            }
            _ => {}
        }
    }

    erase(&term, page)?;

    let selected: Vec<SelectedFile> = items
        .into_iter()
        .filter(|i| i.included)
        .map(|i| SelectedFile {
            path: i.path,
            strategy: i.strategy,
        })
        .collect();

    if selected.is_empty() {
        anyhow::bail!("no files selected");
    }

    Ok(selected)
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

#[must_use]
const fn strategy_label(s: Strategy) -> &'static str {
    match s {
        Strategy::Replace => "replace    ",
        Strategy::CreateOnly => "create_only",
        Strategy::Delete => "delete     ",
        Strategy::Patch => "patch      ",
    }
}

/// Render `page` rows starting at `offset`, plus one footer line.
#[cfg_attr(coverage_nightly, coverage(off))]
fn render(
    term: &Term,
    items: &[Item],
    cursor: usize,
    offset: usize,
    page: usize,
) -> io::Result<()> {
    let end = offset.saturating_add(page).min(items.len());
    for (idx, item) in items
        .iter()
        .enumerate()
        .skip(offset)
        .take(end.saturating_sub(offset))
    {
        let pointer = if idx == cursor { ">" } else { " " };
        let check = if item.included { "✓" } else { " " };
        let strategy = strategy_label(item.strategy);
        let path = &item.path;
        debug_assert_eq!(
            strategy.len(),
            STRATEGY_WIDTH,
            "strategy label width mismatch"
        );
        let row = format!("{pointer} [{check}] {strategy}  {path}");
        if idx == cursor {
            term.write_line(&style(row).bold().to_string())?;
        } else {
            term.write_line(&row)?;
        }
    }

    let pos = cursor.saturating_add(1);
    let total = items.len();
    let selected_count = items.iter().filter(|i| i.included).count();
    let footer = format!("  (item {pos}/{total}, {selected_count} selected)");
    term.write_line(&style(footer).dim().to_string())?;

    Ok(())
}

/// Clear the previously rendered frame and re-render.
#[cfg_attr(coverage_nightly, coverage(off))]
fn redraw(
    term: &Term,
    items: &[Item],
    cursor: usize,
    offset: usize,
    page: usize,
) -> anyhow::Result<()> {
    // page item rows + 1 footer row
    term.clear_last_lines(page.saturating_add(1))
        .context("terminal clear failed")?;
    render(term, items, cursor, offset, page).context("terminal write failed")?;
    Ok(())
}

/// Clear the rendered frame (called on exit).
#[cfg_attr(coverage_nightly, coverage(off))]
fn erase(term: &Term, page: usize) -> anyhow::Result<()> {
    term.clear_last_lines(page.saturating_add(1))
        .context("terminal clear failed")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strategy_labels_have_consistent_width() {
        for &s in STRATEGIES {
            assert_eq!(
                strategy_label(s).len(),
                STRATEGY_WIDTH,
                "strategy label '{s:?}' has wrong width",
            );
        }
    }

    #[test]
    fn strategies_cycle_correctly() {
        let mut s = Strategy::Replace;
        for expected in [
            Strategy::CreateOnly,
            Strategy::Delete,
            Strategy::Patch,
            Strategy::Replace, // wraps around
        ] {
            let idx = STRATEGIES.iter().position(|&x| x == s).unwrap_or(0);
            let next_idx = cycle_next(idx, STRATEGIES.len());
            s = STRATEGIES
                .get(next_idx)
                .copied()
                .unwrap_or(Strategy::Replace);
            assert_eq!(s, expected);
        }
    }
}
