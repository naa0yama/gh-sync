# Plan: GitHub API 422 Guard Improvements

Spec: docs/specs/2026-04-16-api-422-guards-design.md

## Tasks

### Task 1: All-merge-methods-false guard

File: crates/gh-sync-manifest/src/manifest.rs

- Added validation rule in validate_schema() rejecting specs where all three
  merge methods are explicitly false.
- Added 2 tests: all-disabled triggers error, one-enabled passes.

### Task 2: allowed_actions constraints

File: crates/gh-sync-manifest/src/manifest.rs

- Added Rule A: invalid allowed_actions value (not all/local_only/selected)
- Added Rule B: allowed_actions "selected" without populated selected_actions
- Added 4 tests covering both error and valid-pass cases.

### Prior fix (separate commit): apply.rs title/message stripping

File: crates/gh-sync-engine/src/repo/apply.rs

- Strip merge_commit_title/message when allow_merge_commit: false
- Strip squash_commit_title/message when allow_squash_merge: false
- Added 3 tests; test assertions updated to use .get().is_none() pattern.
