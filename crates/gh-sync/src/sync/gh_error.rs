use std::borrow::Cow;

use serde::Deserialize;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single field-level GitHub API error.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct GhApiFieldError {
    pub resource: Option<String>,
    pub field: Option<String>,
    pub code: Option<String>,
    pub message: Option<String>,
}

/// Parsed GitHub API error response.
#[derive(Debug, Clone, Default)]
pub struct GhApiError {
    pub status: Option<u16>,
    pub message: Option<String>,
    pub errors: Vec<GhApiFieldError>,
    #[allow(dead_code)] // included for completeness; not shown in bail messages
    pub documentation_url: Option<String>,
    pub request_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Raw deserialization helper
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawApiError {
    message: Option<String>,
    status: Option<serde_json::Value>,
    errors: Option<serde_json::Value>,
    documentation_url: Option<String>,
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Parse API error from gh stdout/stderr streams.
///
/// stdout is tried first (gh api writes JSON error there on 4xx).
/// Falls back to stderr for plain text gh messages.
pub fn parse_from_streams(stdout: &[u8], stderr: &[u8]) -> Option<GhApiError> {
    // Try stdout JSON first.
    let stdout_result = std::str::from_utf8(stdout)
        .ok()
        .filter(|s| s.contains('{'))
        .and_then(extract_json)
        .and_then(parse_json_error);

    if stdout_result.is_some() {
        return stdout_result;
    }

    // Falls back to stderr JSON, then plain gh message format.
    let stderr_str = std::str::from_utf8(stderr).ok()?;

    let stderr_json = stderr_str
        .contains('{')
        .then(|| extract_json(stderr_str))
        .flatten()
        .and_then(parse_json_error);

    stderr_json.or_else(|| try_parse_gh_message(stderr_str))
}

/// Truncate `s` to the last `max` bytes (character boundary).
///
/// Prepends `"...(truncated {orig} -> {max} bytes)\n"` when truncated.
pub fn truncate_tail(s: &str, max: usize) -> Cow<'_, str> {
    if s.len() <= max {
        return Cow::Borrowed(s);
    }

    // Find a valid UTF-8 boundary at or after `s.len() - max`.
    // saturating_sub is safe because s.len() > max (guard above).
    let start_raw = s.len().saturating_sub(max);
    let start = (start_raw..=s.len())
        .find(|&i| s.is_char_boundary(i))
        .unwrap_or(s.len());

    let tail = &s[start..];
    let header = format!("...(truncated {} -> {} bytes)\n", s.len(), max);
    Cow::Owned(format!("{header}{tail}"))
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

// Extract the leading balanced {...} JSON object from s.
// Returns None if no valid JSON object found.
fn extract_json(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let bytes = s.as_bytes();
    let mut depth: u32 = 0;
    let mut in_string = false;
    let mut escaped = false;

    for (i, &ch) in bytes.iter().enumerate().skip(start) {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == b'\\' && in_string {
            escaped = true;
            continue;
        }
        if ch == b'"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        match ch {
            b'{' => depth = depth.saturating_add(1),
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return s.get(start..=i);
                }
            }
            _ => {}
        }
    }

    None
}

// Infer HTTP status from status_json field (may be number or string like "422")
// and from common substrings in message (e.g. "HTTP 404", "Not Found", "Forbidden").
fn infer_http_status(status_val: Option<&serde_json::Value>, message: &str) -> Option<u16> {
    if let Some(val) = status_val {
        match val {
            serde_json::Value::Number(n) => {
                if let Some(u) = n.as_u64() {
                    return u16::try_from(u).ok();
                }
            }
            serde_json::Value::String(s) => {
                if let Ok(n) = s.parse::<u16>() {
                    return Some(n);
                }
            }
            _ => {}
        }
    }

    // Fall back to message-based inference.
    if message.contains("HTTP 404") {
        Some(404)
    } else if message.contains("HTTP 401") {
        Some(401)
    } else if message.contains("HTTP 403") {
        Some(403)
    } else if message.contains("HTTP 422") {
        Some(422)
    } else if message.contains("Not Found") {
        Some(404)
    } else if message.contains("Unauthorized") {
        Some(401)
    } else if message.contains("Forbidden") {
        Some(403)
    } else if message.contains("Validation Failed") {
        Some(422)
    } else {
        None
    }
}

// Parse the gh CLI human-readable format: "gh: <detail> (Status Label)"
// e.g. "gh: Validation Failed (HTTP 422)"
fn try_parse_gh_message(stderr: &str) -> Option<GhApiError> {
    // rfind is used so nested parens like "(detail) (HTTP 422)" correctly pick
    // up the outermost label at the end of the line.
    let last_open = stderr.rfind('(')?;
    let last_close = stderr.rfind(')')?;
    if last_close <= last_open {
        return None;
    }

    let label = stderr.get(last_open.saturating_add(1)..last_close)?;

    // Extract detail between "gh: " and the label parenthesis.
    let detail = stderr
        .find("gh: ")
        .map_or(stderr, |idx| &stderr[idx.saturating_add(4)..]);
    let detail = detail
        .rfind('(')
        .map_or(detail, |idx| detail[..idx].trim_end());

    let status = infer_http_status(None, label)?;

    Some(GhApiError {
        status: Some(status),
        message: Some(label.to_owned()),
        errors: if detail.is_empty() || detail == label {
            vec![]
        } else {
            vec![GhApiFieldError {
                message: Some(detail.to_owned()),
                ..Default::default()
            }]
        },
        documentation_url: None,
        request_id: None,
    })
}

// Parse a JSON string slice into a GhApiError. Returns None if the message is empty.
fn parse_json_error(json_str: &str) -> Option<GhApiError> {
    let Ok(raw) = serde_json::from_str::<RawApiError>(json_str) else {
        return None;
    };
    let message = raw.message.filter(|m| !m.is_empty())?;

    let field_errors = match raw.errors {
        Some(serde_json::Value::Array(arr)) => {
            let as_objects: Result<Vec<GhApiFieldError>, _> =
                serde_json::from_value(serde_json::Value::Array(arr.clone()));
            as_objects.unwrap_or_else(|_| {
                let as_strings: Result<Vec<String>, _> =
                    serde_json::from_value(serde_json::Value::Array(arr));
                as_strings
                    .unwrap_or_default()
                    .into_iter()
                    .map(|s| GhApiFieldError {
                        message: Some(s),
                        ..Default::default()
                    })
                    .collect()
            })
        }
        _ => vec![],
    };

    let status = infer_http_status(raw.status.as_ref(), &message);

    Some(GhApiError {
        status,
        message: Some(message),
        errors: field_errors,
        documentation_url: raw.documentation_url,
        request_id: None,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn parse_from_streams_prefers_stdout_json() {
        let stdout = br#"{"message":"stdout error","status":"404"}"#;
        let stderr = b"gh: some other error (HTTP 422)";
        let result = parse_from_streams(stdout, stderr).unwrap();
        assert_eq!(result.message.as_deref(), Some("stdout error"));
    }

    #[test]
    fn parse_from_streams_extracts_field_level_errors() {
        let stdout = br#"{"message":"Validation Failed","errors":[{"resource":"Repository","field":"merge_commit_title","code":"invalid"}],"documentation_url":"https://docs.github.com","status":"422"}"#;
        let result = parse_from_streams(stdout, b"").unwrap();
        assert_eq!(
            result.errors[0].field.as_deref(),
            Some("merge_commit_title")
        );
    }

    #[test]
    fn parse_from_streams_falls_back_to_stderr() {
        let stdout = b"";
        let stderr = br#"{"message":"stderr error","status":"403"}"#;
        let result = parse_from_streams(stdout, stderr).unwrap();
        assert_eq!(result.message.as_deref(), Some("stderr error"));
    }

    #[test]
    fn parse_from_streams_infers_status_from_gh_prefix_text() {
        let stdout = b"";
        let stderr = b"gh: Validation Failed (HTTP 422)";
        let result = parse_from_streams(stdout, stderr).unwrap();
        assert_eq!(result.status, Some(422));
    }

    #[test]
    fn parse_from_streams_handles_string_errors_array() {
        let stdout = br#"{"message":"Something went wrong","errors":["some error string"]}"#;
        let result = parse_from_streams(stdout, b"").unwrap();
        assert_eq!(
            result.errors[0].message.as_deref(),
            Some("some error string")
        );
    }

    #[test]
    fn truncate_tail_short_string_unchanged() {
        let s = "hello";
        let result = truncate_tail(s, 10);
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result.as_ref(), "hello");
    }

    #[test]
    fn truncate_tail_long_string_truncated() {
        let s = "abcdefghijklmnopqrstuvwxyz";
        let result = truncate_tail(s, 10);
        assert!(matches!(result, Cow::Owned(_)));
        let r = result.as_ref();
        assert!(r.contains("truncated"));
        assert!(r.ends_with("qrstuvwxyz"));
    }

    #[test]
    fn infer_status_from_http_prefix() {
        assert_eq!(infer_http_status(None, "something (HTTP 404)"), Some(404));
    }

    #[test]
    fn infer_status_from_branch_not_protected() {
        assert_eq!(
            infer_http_status(None, "Branch not protected: Not Found"),
            Some(404)
        );
    }

    #[test]
    fn infer_status_from_422_validation() {
        assert_eq!(infer_http_status(None, "Validation Failed"), Some(422));
    }
}
