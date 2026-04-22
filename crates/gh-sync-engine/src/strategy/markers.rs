//! Marker-block stripping and restoration for `preserve_markers` mode.
//!
//! Lines containing `gh-sync:keep-start` / `gh-sync:keep-end` tokens delimit
//! protected blocks. The comment character (`#`, `//`, etc.) is irrelevant;
//! only the presence of the token in the line is checked.

use std::fmt;

/// Token that opens a preserved block.
pub const MARKER_START: &str = "gh-sync:keep-start";
/// Token that closes a preserved block.
pub const MARKER_END: &str = "gh-sync:keep-end";

/// A protected block extracted by [`strip_marker_blocks`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerBlock {
    /// Number of non-marker lines in the stripped output that precede this block.
    pub position: usize,
    /// Raw bytes of the block, including the marker comment lines and their content.
    pub content: Vec<u8>,
}

/// Errors produced by marker parsing.
#[derive(Debug, PartialEq, Eq)]
pub enum MarkerError {
    /// A `keep-start` without a closing `keep-end`, or a `keep-end` without a preceding `keep-start`.
    Unbalanced,
    /// A `keep-start` encountered inside an already-open block.
    Nested,
}

impl fmt::Display for MarkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unbalanced => write!(
                f,
                "unbalanced gh-sync marker (keep-start / keep-end mismatch)"
            ),
            Self::Nested => write!(f, "nested gh-sync markers are not allowed"),
        }
    }
}

impl std::error::Error for MarkerError {}

fn line_contains(line: &[u8], token: &str) -> bool {
    let tok = token.as_bytes();
    line.windows(tok.len()).any(|w| w == tok)
}

/// Remove marker blocks from `bytes`.
///
/// Returns `(stripped, blocks)` where `stripped` is the content with all
/// marker lines and their enclosed content removed, and `blocks` carries
/// enough information to reconstruct the original via [`merge_marker_blocks`].
///
/// # Errors
///
/// Returns [`MarkerError::Nested`] when a `keep-start` is found inside an
/// open block, or [`MarkerError::Unbalanced`] when open/close counts differ.
pub fn strip_marker_blocks(bytes: &[u8]) -> Result<(Vec<u8>, Vec<MarkerBlock>), MarkerError> {
    let mut output: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut blocks: Vec<MarkerBlock> = Vec::new();
    let mut stripped_line_count: usize = 0;

    let mut in_block = false;
    let mut block_position: usize = 0;
    let mut block_content: Vec<u8> = Vec::new();

    for line in bytes.split_inclusive(|&b| b == b'\n') {
        if line_contains(line, MARKER_START) {
            if in_block {
                return Err(MarkerError::Nested);
            }
            in_block = true;
            block_position = stripped_line_count;
            block_content.clear();
            block_content.extend_from_slice(line);
        } else if line_contains(line, MARKER_END) {
            if !in_block {
                return Err(MarkerError::Unbalanced);
            }
            block_content.extend_from_slice(line);
            blocks.push(MarkerBlock {
                position: block_position,
                content: std::mem::take(&mut block_content),
            });
            in_block = false;
        } else if in_block {
            block_content.extend_from_slice(line);
        } else {
            output.extend_from_slice(line);
            stripped_line_count = stripped_line_count.saturating_add(1);
        }
    }

    if in_block {
        return Err(MarkerError::Unbalanced);
    }

    Ok((output, blocks))
}

/// Choose which marker blocks to use when merging.
///
/// Returns `local_blocks` when it is non-empty (downstream has its own
/// customisations), otherwise falls back to `upstream_blocks` so that the
/// upstream template markers are propagated to a downstream that has not yet
/// added any markers of its own.
#[must_use]
pub fn select_marker_blocks(
    upstream_blocks: Vec<MarkerBlock>,
    local_blocks: Vec<MarkerBlock>,
) -> Vec<MarkerBlock> {
    if local_blocks.is_empty() {
        upstream_blocks
    } else {
        local_blocks
    }
}

/// Reinsert extracted marker blocks into stripped bytes.
///
/// This is the inverse of [`strip_marker_blocks`]: given the stripped content
/// and the blocks it returned, this function reconstructs the original bytes.
///
/// Blocks are inserted in the order they appear in the slice (ascending
/// `position`). Passing out-of-order blocks produces undefined output.
#[must_use]
pub fn merge_marker_blocks(stripped: &[u8], blocks: &[MarkerBlock]) -> Vec<u8> {
    if blocks.is_empty() {
        return stripped.to_vec();
    }

    let lines: Vec<&[u8]> = stripped.split_inclusive(|&b| b == b'\n').collect();
    let extra: usize = blocks.iter().map(|b| b.content.len()).sum();
    let mut result: Vec<u8> = Vec::with_capacity(stripped.len().saturating_add(extra));

    let mut cursor: usize = 0;

    for block in blocks {
        let until = block.position.min(lines.len());
        let count = until.saturating_sub(cursor);
        for line in lines.iter().skip(cursor).take(count) {
            result.extend_from_slice(line);
        }
        cursor = until;
        result.extend_from_slice(&block.content);
    }

    for line in lines.iter().skip(cursor) {
        result.extend_from_slice(line);
    }

    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::indexing_slicing)]

    use super::*;

    // ------------------------------------------------------------------
    // strip_marker_blocks — basic round-trip
    // ------------------------------------------------------------------

    #[test]
    fn strip_single_block_at_start() {
        let input =
            b"# gh-sync:keep-start\nversion = \"0.2.1\"\n# gh-sync:keep-end\nother = true\n";
        let (stripped, blocks) = strip_marker_blocks(input).unwrap();
        assert_eq!(stripped, b"other = true\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].position, 0);
        assert_eq!(
            blocks[0].content,
            b"# gh-sync:keep-start\nversion = \"0.2.1\"\n# gh-sync:keep-end\n"
        );
    }

    #[test]
    fn strip_single_block_in_middle() {
        let input = b"a = 1\nb = 2\n# gh-sync:keep-start\nc = 3\n# gh-sync:keep-end\nd = 4\n";
        let (stripped, blocks) = strip_marker_blocks(input).unwrap();
        assert_eq!(stripped, b"a = 1\nb = 2\nd = 4\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].position, 2);
    }

    #[test]
    fn strip_two_blocks() {
        let input = b"# gh-sync:keep-start\nA\n# gh-sync:keep-end\nmid\n# gh-sync:keep-start\nB\n# gh-sync:keep-end\nend\n";
        let (stripped, blocks) = strip_marker_blocks(input).unwrap();
        assert_eq!(stripped, b"mid\nend\n");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].position, 0);
        assert_eq!(blocks[1].position, 1);
    }

    #[test]
    fn strip_no_markers_returns_input() {
        let input = b"a = 1\nb = 2\n";
        let (stripped, blocks) = strip_marker_blocks(input).unwrap();
        assert_eq!(stripped, input);
        assert!(blocks.is_empty());
    }

    // ------------------------------------------------------------------
    // merge_marker_blocks — round-trip
    // ------------------------------------------------------------------

    #[test]
    fn merge_round_trips_single_block() {
        let input =
            b"# gh-sync:keep-start\nversion = \"0.2.1\"\n# gh-sync:keep-end\nother = true\n";
        let (stripped, blocks) = strip_marker_blocks(input).unwrap();
        let restored = merge_marker_blocks(&stripped, &blocks);
        assert_eq!(restored, input);
    }

    #[test]
    fn merge_round_trips_two_blocks() {
        let input = b"# gh-sync:keep-start\nA\n# gh-sync:keep-end\nmid\n# gh-sync:keep-start\nB\n# gh-sync:keep-end\nend\n";
        let (stripped, blocks) = strip_marker_blocks(input).unwrap();
        let restored = merge_marker_blocks(&stripped, &blocks);
        assert_eq!(restored, input);
    }

    #[test]
    fn merge_empty_blocks_returns_stripped() {
        let input = b"a = 1\n";
        let restored = merge_marker_blocks(input, &[]);
        assert_eq!(restored, input);
    }

    // ------------------------------------------------------------------
    // Error cases
    // ------------------------------------------------------------------

    #[test]
    fn orphan_start_returns_unbalanced() {
        let input = b"# gh-sync:keep-start\na = 1\n";
        assert_eq!(strip_marker_blocks(input), Err(MarkerError::Unbalanced));
    }

    #[test]
    fn orphan_end_returns_unbalanced() {
        let input = b"a = 1\n# gh-sync:keep-end\n";
        assert_eq!(strip_marker_blocks(input), Err(MarkerError::Unbalanced));
    }

    #[test]
    fn nested_start_returns_nested() {
        let input =
            b"# gh-sync:keep-start\n# gh-sync:keep-start\ninner\n# gh-sync:keep-end\n# gh-sync:keep-end\n";
        assert_eq!(strip_marker_blocks(input), Err(MarkerError::Nested));
    }

    // ------------------------------------------------------------------
    // select_marker_blocks
    // ------------------------------------------------------------------

    fn make_block(position: usize, content: &[u8]) -> MarkerBlock {
        MarkerBlock {
            position,
            content: content.to_vec(),
        }
    }

    #[test]
    fn select_local_empty_returns_upstream() {
        let upstream = vec![make_block(
            0,
            b"# gh-sync:keep-start\na\n# gh-sync:keep-end\n",
        )];
        let local: Vec<MarkerBlock> = vec![];
        let result = select_marker_blocks(upstream.clone(), local);
        assert_eq!(result, upstream);
    }

    #[test]
    fn select_local_non_empty_returns_local() {
        let upstream = vec![make_block(
            0,
            b"# gh-sync:keep-start\nupstream\n# gh-sync:keep-end\n",
        )];
        let local = vec![make_block(
            0,
            b"# gh-sync:keep-start\nlocal\n# gh-sync:keep-end\n",
        )];
        let result = select_marker_blocks(upstream, local.clone());
        assert_eq!(result, local);
    }

    #[test]
    fn select_both_empty_returns_empty() {
        let result = select_marker_blocks(vec![], vec![]);
        assert!(result.is_empty());
    }

    // ------------------------------------------------------------------
    // Comment style independence
    // ------------------------------------------------------------------

    #[test]
    fn jsonc_style_markers_are_recognised() {
        let input = b"{\n// gh-sync:keep-start\n\"name\": \"gh-sync\"\n// gh-sync:keep-end\n}\n";
        let (stripped, blocks) = strip_marker_blocks(input).unwrap();
        assert_eq!(stripped, b"{\n}\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].position, 1);
        let restored = merge_marker_blocks(&stripped, &blocks);
        assert_eq!(restored, input);
    }
}
