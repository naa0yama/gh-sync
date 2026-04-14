use super::StrategyResult;

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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::panic)]

    use super::*;

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
