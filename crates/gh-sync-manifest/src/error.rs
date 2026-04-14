use std::fmt;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// ValidationError
// ---------------------------------------------------------------------------

/// A single schema or reference validation failure.
#[derive(Debug, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)] // "ValidationError" in module "error" is intentional
pub struct ValidationError {
    /// Zero-based index of the rule that failed, or `None` for top-level errors.
    pub rule_index: Option<usize>,
    /// The field name that violated a constraint (e.g. `"path"`, `"strategy"`).
    pub field: String,
    /// Human-readable description of the violation.
    pub message: String,
}

impl ValidationError {
    /// Create a top-level (non-rule) validation error.
    pub fn top_level(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            rule_index: None,
            field: field.into(),
            message: message.into(),
        }
    }

    /// Create a rule-level validation error for the rule at `index`.
    pub fn rule(index: usize, field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            rule_index: Some(index),
            field: field.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.rule_index {
            None => write!(f, "{}: {}", self.field, self.message),
            Some(i) => write!(f, "files[{}].{}: {}", i, self.field, self.message),
        }
    }
}

// ---------------------------------------------------------------------------
// SyncError
// ---------------------------------------------------------------------------

/// Top-level error type for the `sync` subcommand.
#[derive(Debug)]
#[allow(clippy::module_name_repetitions)] // "SyncError" named for clarity at call sites
pub enum SyncError {
    /// The manifest file could not be read or parsed.
    ManifestLoad {
        /// Path to the manifest file.
        path: PathBuf,
        /// Underlying I/O or parse error.
        source: anyhow::Error,
    },
    /// One or more validation errors were found.
    Validation(Vec<ValidationError>),
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ManifestLoad { path, source } => {
                write!(f, "failed to load manifest {}: {source}", path.display())
            }
            Self::Validation(errors) => {
                write!(f, "{} validation error(s):", errors.len())?;
                for e in errors {
                    write!(f, "\n  - {e}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for SyncError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ManifestLoad { source, .. } => source.source(),
            Self::Validation(_) => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::panic)]

    use super::*;

    // ------------------------------------------------------------------
    // ValidationError::Display
    // ------------------------------------------------------------------

    #[test]
    fn validation_error_display_top_level() {
        // Arrange
        let err = ValidationError::top_level("upstream.repo", "must not be empty");

        // Act
        let msg = err.to_string();

        // Assert
        assert_eq!(msg, "upstream.repo: must not be empty");
    }

    #[test]
    fn validation_error_display_rule_level() {
        // Arrange
        let err = ValidationError::rule(2, "strategy", "unsupported value");

        // Act
        let msg = err.to_string();

        // Assert
        assert_eq!(msg, "files[2].strategy: unsupported value");
    }

    // ------------------------------------------------------------------
    // SyncError::Display
    // ------------------------------------------------------------------

    #[test]
    fn sync_error_manifest_load_display() {
        // Arrange
        let err = SyncError::ManifestLoad {
            path: PathBuf::from(".github/gh-sync/config.yml"),
            source: anyhow::anyhow!("No such file or directory"),
        };

        // Act
        let msg = err.to_string();

        // Assert
        assert!(
            msg.contains("failed to load manifest"),
            "missing prefix: {msg}"
        );
        assert!(
            msg.contains(".github/gh-sync/config.yml"),
            "missing path: {msg}"
        );
        assert!(
            msg.contains("No such file or directory"),
            "missing cause: {msg}"
        );
    }

    #[test]
    fn sync_error_validation_display_single() {
        // Arrange
        let err = SyncError::Validation(vec![ValidationError::top_level(
            "files",
            "must not be empty",
        )]);

        // Act
        let msg = err.to_string();

        // Assert
        assert!(
            msg.contains("1 validation error(s):"),
            "missing count: {msg}"
        );
        assert!(
            msg.contains("files: must not be empty"),
            "missing detail: {msg}"
        );
    }

    #[test]
    fn sync_error_validation_display_multiple() {
        // Arrange
        let err = SyncError::Validation(vec![
            ValidationError::top_level("upstream.repo", "invalid format"),
            ValidationError::rule(0, "path", "must not be empty"),
        ]);

        // Act
        let msg = err.to_string();

        // Assert
        assert!(
            msg.contains("2 validation error(s):"),
            "missing count: {msg}"
        );
        assert!(
            msg.contains("upstream.repo: invalid format"),
            "missing first: {msg}"
        );
        assert!(
            msg.contains("files[0].path: must not be empty"),
            "missing second: {msg}"
        );
    }

    // ------------------------------------------------------------------
    // SyncError::source
    // ------------------------------------------------------------------

    #[test]
    fn sync_error_manifest_load_source_returns_none_for_non_chain() {
        // Arrange — anyhow errors without a root cause have no source
        let err = SyncError::ManifestLoad {
            path: PathBuf::from("x.yml"),
            source: anyhow::anyhow!("leaf error"),
        };

        // Act / Assert
        assert!(
            std::error::Error::source(&err).is_none(),
            "leaf anyhow error has no chained source"
        );
    }

    #[test]
    fn sync_error_validation_source_is_none() {
        // Arrange
        let err = SyncError::Validation(vec![]);

        // Act / Assert
        assert!(
            std::error::Error::source(&err).is_none(),
            "Validation variant has no source"
        );
    }
}
