use std::path::Path;

/// Default output path for the generated skill file.
pub const SKILL_PATH: &str = ".claude/skills/gh-sync/SKILL.md";

/// Render the skill Markdown content, substituting the current package version.
#[must_use]
pub fn render() -> String {
    let version = concat!("v", env!("CARGO_PKG_VERSION"));
    TEMPLATE.replace("{{version}}", version)
}

/// Write pre-rendered `content` to `path`, creating parent directories as needed.
///
/// # Errors
///
/// Returns an error when the directory cannot be created or the file cannot be written.
pub fn write_skill_from_content(path: &Path, content: &str) -> anyhow::Result<()> {
    super::write_file(path, content)
}

const TEMPLATE: &str = "\
---
name: gh-sync
description: >
  gh-sync {{version}} — marker block usage and preserve_markers configuration
---

# gh-sync Marker Block Usage

gh-sync supports `preserve_markers` to protect downstream-specific content
from being overwritten during template sync.

## Marker Syntax

Enclose downstream-specific lines between `gh-sync:keep-start` and
`gh-sync:keep-end` tokens. The comment character (`#`, `//`, etc.) is
irrelevant — only the token presence in the line is checked.

TOML / YAML / shell (`#` comment style):

```toml
# gh-sync:keep-start
version = \"0.3.0\"
# gh-sync:keep-end
```

JSON / JSONC (`//` comment style):

```jsonc
// gh-sync:keep-start
\"name\": \"my-downstream-project\"
// gh-sync:keep-end
```

## Enabling in the Manifest

Add `preserve_markers: true` to each `strategy: patch` rule that uses marker
blocks. This field is set in the upstream `gh-sync.yaml` manifest, not in a
local overlay.

```yaml
files:
  - path: Cargo.toml
    strategy: patch
    preserve_markers: true
```

## Behavior

- Before `unified_diff` / `patch apply`, **both** upstream and local bytes have
  marker blocks stripped. Only non-marker content is compared or patched.
- On sync write-back, marker blocks from the local file are preserved as-is;
  only non-marker regions are updated from upstream.
- `gh-sync sync --patch-refresh` regenerates patch files excluding marker content.

## Error Conditions

| Error | Cause | Fix |
| ----- | ----- | --- |
| `unbalanced gh-sync marker` | `keep-start` without matching `keep-end` (or vice versa) | Add the missing marker line |
| `nested gh-sync markers are not allowed` | `keep-start` found inside an already-open block | Remove the inner `keep-start` |

## Common Use Cases

### Cargo.toml — workspace version

```toml
[workspace.package]
# gh-sync:keep-start
version = \"0.2.1\"
# gh-sync:keep-end
```

### mise.toml — environment variable overrides

```toml
[env]
# gh-sync:keep-start
RUST_LOG = \"warn,gh_sync=trace\"
# gh-sync:keep-end
```

### .vscode/launch.json — project-specific launch configuration

```jsonc
{
  // gh-sync:keep-start
  \"name\": \"my-binary\",
  \"cargo\": { \"args\": [\"build\", \"--bin\", \"my-binary\"] }
  // gh-sync:keep-end
}
```

## Notes

- `preserve_markers: true` is opt-in per rule. Rules without it treat marker
  comment lines as ordinary content.
- Nested marker blocks are not supported and produce an error.
- Use `strategy: ignore` to skip syncing an entire file rather than
  protecting specific regions within it.
";

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn render_starts_with_frontmatter() {
        let out = render();
        assert!(
            out.starts_with("---\n"),
            "skill must start with YAML frontmatter"
        );
    }

    #[test]
    fn render_contains_marker_tokens() {
        let out = render();
        assert!(
            out.contains("gh-sync:keep-start"),
            "missing keep-start token"
        );
        assert!(out.contains("gh-sync:keep-end"), "missing keep-end token");
    }

    #[test]
    fn render_contains_preserve_markers_config() {
        let out = render();
        assert!(
            out.contains("preserve_markers: true"),
            "missing preserve_markers config"
        );
    }

    #[test]
    fn render_contains_both_comment_styles() {
        let out = render();
        assert!(
            out.contains("# gh-sync:keep-start"),
            "missing # comment style"
        );
        assert!(
            out.contains("// gh-sync:keep-start"),
            "missing // comment style"
        );
    }

    #[test]
    fn render_substitutes_version() {
        let out = render();
        assert!(
            !out.contains("{{version}}"),
            "version placeholder not replaced"
        );
        assert!(
            out.contains('v'),
            "rendered output should contain version with v prefix"
        );
    }

    #[test]
    fn write_skill_from_content_creates_file_and_dirs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".claude/skills/gh-sync/SKILL.md");
        write_skill_from_content(&path, "test content").unwrap();
        let read_back = std::fs::read_to_string(&path).unwrap();
        assert_eq!(read_back, "test content");
    }

    #[test]
    fn write_skill_from_content_overwrites_existing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("SKILL.md");
        std::fs::write(&path, b"old").unwrap();
        write_skill_from_content(&path, "new content").unwrap();
        let read_back = std::fs::read_to_string(&path).unwrap();
        assert_eq!(read_back, "new content");
    }
}
