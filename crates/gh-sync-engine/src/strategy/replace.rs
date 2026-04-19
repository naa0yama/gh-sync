use super::StrategyResult;
use super::markers::{merge_marker_blocks, strip_marker_blocks};

/// Apply the `replace` strategy.
///
/// Writes `upstream` content to the local path, unless the local file already
/// has identical content.
#[must_use]
pub fn apply(upstream: &[u8], local: Option<&[u8]>) -> StrategyResult {
    match local {
        Some(existing) if existing == upstream => StrategyResult::Unchanged,
        _ => StrategyResult::Changed {
            content: upstream.to_vec(),
        },
    }
}

/// Apply the `replace` strategy with marker-block preservation.
///
/// Strips marker blocks from both sides, replaces non-marker content with
/// upstream, then re-inserts the local marker blocks into the result.
#[must_use]
pub fn apply_with_markers(upstream: &[u8], local: Option<&[u8]>) -> StrategyResult {
    let upstream_stripped = match strip_marker_blocks(upstream) {
        Ok((s, _)) => s,
        Err(e) => {
            return StrategyResult::Error(format!("invalid marker block (upstream): {e}"));
        }
    };
    let local_blocks = match local {
        Some(b) => match strip_marker_blocks(b) {
            Ok((_, blocks)) => blocks,
            Err(e) => {
                return StrategyResult::Error(format!("invalid marker block (local): {e}"));
            }
        },
        None => Vec::new(),
    };
    let merged = merge_marker_blocks(&upstream_stripped, &local_blocks);
    if local == Some(merged.as_slice()) {
        StrategyResult::Unchanged
    } else {
        StrategyResult::Changed { content: merged }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::panic)]

    use super::*;

    // ------------------------------------------------------------------
    // apply_with_markers
    // ------------------------------------------------------------------

    #[test]
    fn apply_with_markers_local_none_returns_changed() {
        let upstream = b"a = 1\nb = 2\n";
        let result = apply_with_markers(upstream, None);
        assert!(
            matches!(result, StrategyResult::Changed { ref content } if content == upstream),
            "expected Changed when local is None"
        );
    }

    #[test]
    fn apply_with_markers_no_markers_matches_apply() {
        // Without marker blocks, apply_with_markers must behave like apply.
        let upstream = b"new\n";
        let local = b"old\n";
        let result = apply_with_markers(upstream, Some(local));
        assert!(
            matches!(result, StrategyResult::Changed { ref content } if content == upstream),
            "expected Changed equal to upstream when no markers"
        );
    }

    #[test]
    fn apply_with_markers_local_blocks_preserved_when_upstream_changed() {
        // Arrange: upstream has no markers; local wraps a value in a marker block.
        let upstream = b"a = upstream\n";
        let local = b"a = upstream\n# gh-sync:keep-start\nb = local\n# gh-sync:keep-end\n";
        // After strip(upstream) = "a = upstream\n"; local blocks = the b=local block.
        // merged = "a = upstream\n" + block → same as local → Unchanged.
        let result = apply_with_markers(upstream, Some(local));
        assert!(
            matches!(result, StrategyResult::Unchanged),
            "expected Unchanged when only marker blocks differ from bare upstream"
        );
    }

    #[test]
    fn apply_with_markers_changed_when_non_marker_differs() {
        let upstream = b"a = new\n";
        let marker_block = b"# gh-sync:keep-start\nb = local\n# gh-sync:keep-end\n";
        let local = [b"a = old\n".as_slice(), marker_block.as_slice()].concat();
        let result = apply_with_markers(upstream, Some(&local));
        // expected content = upstream + marker block
        let expected: Vec<u8> = [b"a = new\n".as_slice(), marker_block.as_slice()].concat();
        assert!(
            matches!(result, StrategyResult::Changed { ref content } if *content == expected),
            "expected Changed with upstream content plus local marker block"
        );
    }

    #[test]
    fn apply_with_markers_unchanged_when_stripped_equal() {
        // local == merge_marker_blocks(upstream_stripped, local_blocks) → Unchanged
        let upstream = b"a = 1\n";
        let local = b"a = 1\n";
        let result = apply_with_markers(upstream, Some(local));
        assert!(
            matches!(result, StrategyResult::Unchanged),
            "expected Unchanged when content already matches"
        );
    }

    #[test]
    fn apply_with_markers_unbalanced_local_returns_error() {
        let upstream = b"a = 1\n";
        let local = b"# gh-sync:keep-start\na = 1\n"; // missing keep-end
        let result = apply_with_markers(upstream, Some(local));
        assert!(
            matches!(result, StrategyResult::Error(ref msg) if msg.contains("local")),
            "expected Error for unbalanced local markers"
        );
    }

    #[test]
    fn apply_with_markers_unbalanced_upstream_returns_error() {
        let upstream = b"# gh-sync:keep-start\na = 1\n"; // missing keep-end
        let local = b"a = 1\n";
        let result = apply_with_markers(upstream, Some(local));
        assert!(
            matches!(result, StrategyResult::Error(ref msg) if msg.contains("upstream")),
            "expected Error for unbalanced upstream markers"
        );
    }

    #[test]
    fn local_none_returns_changed() {
        // Arrange
        let upstream = b"new content";

        // Act
        let result = apply(upstream, None);

        // Assert
        assert!(
            matches!(result, StrategyResult::Changed { ref content } if content == upstream),
            "expected Changed when local is None"
        );
    }

    #[test]
    fn local_differs_returns_changed() {
        // Arrange
        let upstream = b"new content";
        let local = b"old content";

        // Act
        let result = apply(upstream, Some(local));

        // Assert
        assert!(
            matches!(result, StrategyResult::Changed { ref content } if content == upstream),
            "expected Changed when local differs"
        );
    }

    #[test]
    fn local_matches_returns_unchanged() {
        // Arrange
        let content = b"same content";

        // Act
        let result = apply(content, Some(content));

        // Assert
        assert!(
            matches!(result, StrategyResult::Unchanged),
            "expected Unchanged when local matches upstream"
        );
    }

    #[test]
    fn empty_upstream_local_none_returns_changed() {
        // Arrange / Act
        let result = apply(b"", None);

        // Assert
        assert!(
            matches!(result, StrategyResult::Changed { ref content } if content.is_empty()),
            "expected Changed with empty content when local is None"
        );
    }

    #[test]
    fn empty_upstream_empty_local_returns_unchanged() {
        // Arrange / Act
        let result = apply(b"", Some(b""));

        // Assert
        assert!(
            matches!(result, StrategyResult::Unchanged),
            "expected Unchanged when both upstream and local are empty"
        );
    }
}
