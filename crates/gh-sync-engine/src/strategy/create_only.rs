use super::StrategyResult;

/// Apply the `create_only` strategy.
///
/// Writes `upstream` content to the local path only when the file does not yet
/// exist.  If the file already exists, the rule is skipped without modification.
#[must_use]
pub fn apply(upstream: &[u8], local_exists: bool) -> StrategyResult {
    if local_exists {
        StrategyResult::Skipped {
            reason: String::from("file already exists"),
        }
    } else {
        StrategyResult::Changed {
            content: upstream.to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::panic)]

    use super::*;

    #[test]
    fn file_not_exists_returns_changed() {
        // Arrange
        let upstream = b"initial content";

        // Act
        let result = apply(upstream, false);

        // Assert
        assert!(
            matches!(result, StrategyResult::Changed { ref content } if content == upstream),
            "expected Changed when file does not exist"
        );
    }

    #[test]
    fn file_exists_returns_skipped() {
        // Arrange
        let upstream = b"new upstream content";

        // Act
        let result = apply(upstream, true);

        // Assert
        assert!(
            matches!(result, StrategyResult::Skipped { ref reason } if reason == "file already exists"),
            "expected Skipped when file already exists"
        );
    }

    #[test]
    fn empty_upstream_not_exists_returns_changed_empty() {
        // Arrange / Act
        let result = apply(b"", false);

        // Assert
        assert!(
            matches!(result, StrategyResult::Changed { ref content } if content.is_empty()),
            "expected Changed with empty content"
        );
    }
}
