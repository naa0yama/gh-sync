# Design: GitHub API 422 Guard Improvements

## Problem

`gh-sync sync repo` can produce HTTP 422 (Validation Failed) errors from the
GitHub API when the PATCH payload contains field combinations that GitHub
rejects. One case (disabling a merge method while changing its title/message
fields in the same request) was fixed in a prior commit. This spec covers the
remaining high-priority cases.

## Goals

- Prevent HTTP 422 when the spec disables all three merge methods at once.
- Prevent HTTP 422 when `allowed_actions: selected` is set without any
  `selected_actions` defined.
- Prevent silent typos in `allowed_actions` from reaching the GitHub API.
- Detect all three conditions at manifest-validation time, before any API call.

## Non-Goals

- `archived: true` combined with other fields: no confirmed GitHub API
  constraint found; deferred pending evidence.
- `required_status_checks` with empty contexts: GitHub acceptance unclear;
  deferred pending evidence.
- Runtime checks against live repository state (edge cases where two methods
  are already false in the live repo are not addressed).

## Approach

All three fixes are added to `validate_schema()` in
`crates/gh-sync-manifest/src/manifest.rs`. This function already runs before
any API call, returns structured `ValidationError` items, and is covered by
existing tests — making it the lowest-risk insertion point.

No changes to `apply_core_fields()` are needed for these cases (the merge
method guard fits naturally as a spec-level constraint).

## Implementation Notes

### Task 1 — All merge methods disabled (Issue #1)

File: `crates/gh-sync-manifest/src/manifest.rs` — `validate_schema()`

If `merge_strategy` has all three of `allow_merge_commit`,
`allow_squash_merge`, and `allow_rebase_merge` explicitly set to `false`,
push a `ValidationError::top_level` on field
`"spec.merge_strategy"` with message
`"at least one merge method must be enabled"`.

### Task 2 — `allowed_actions` constraints (Issues #3 and #4)

File: `crates/gh-sync-manifest/src/manifest.rs` — `validate_schema()`

**Issue #4 (invalid value):** If `actions.allowed_actions` is present but
not one of `"all"`, `"local_only"`, `"selected"`, push a
`ValidationError::top_level` on field `"spec.actions.allowed_actions"`.

**Issue #3 (selected without patterns):** If `actions.allowed_actions` is
`"selected"` and `actions.selected_actions` is `None` (or both
`github_owned_allowed` and `patterns_allowed` are `None`), push a
`ValidationError::top_level` on field `"spec.actions.selected_actions"`.

## Testing Strategy

Each new validation path gets at least two unit tests in
`crates/gh-sync-manifest/src/manifest.rs` (inline `#[cfg(test)]` block):

- One test that triggers the new error.
- One test that confirms a valid spec passes.

Run `mise run test` and `mise run pre-commit` to verify.

## Open Questions

None.
