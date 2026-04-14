use super::StrategyResult;

/// Apply the `delete` strategy.
///
/// Marks the local file for deletion when it exists.  If the file is already
/// absent the rule is skipped without error.
#[must_use]
pub fn apply(local_exists: bool) -> StrategyResult {
    if local_exists {
        StrategyResult::Deleted
    } else {
        StrategyResult::Skipped {
            reason: String::from("file does not exist"),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::panic)]

    use super::*;

    #[test]
    fn file_exists_returns_deleted() {
        // Act
        let result = apply(true);

        // Assert
        assert!(
            matches!(result, StrategyResult::Deleted),
            "expected Deleted when file exists"
        );
    }

    #[test]
    fn file_not_exists_returns_skipped() {
        // Act
        let result = apply(false);

        // Assert
        assert!(
            matches!(result, StrategyResult::Skipped { ref reason } if reason == "file does not exist"),
            "expected Skipped when file does not exist"
        );
    }
}
