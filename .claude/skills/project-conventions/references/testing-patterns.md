## Miri Compatibility

For universal Miri rules and decision flowchart, see
`~/.claude/skills/rust-implementation/references/testing.md` → "Miri" section.

### Crate-Level Exclusions

| Crate | Reason                       | Tests |
| ----- | ---------------------------- | ----- |
| none  | All crates are Miri-eligible | —     |

### Per-Test Skip Categories

1. **Process spawning / diff binary (`tempfile` + `std::process::Command`)** — ~4 tests.
   `unified_diff` in `diff.rs` spawns the system `diff` binary and uses
   `tempfile::tempdir()` for temporary files. Both are unsupported under Miri.

2. **File system + process spawning (`tempfile` + patch runner)** — ~4 tests.
   Tests in `mode/patch_refresh.rs` call the real patch runner which spawns
   external processes and writes temp files.

3. **Patch strategy tests (`sync/strategy/patch.rs`)** — 2 tests.
   Tests that invoke the `patch` binary via `std::process::Command`.

4. **Integration tests / process spawning (`assert_cmd`)** — 23 tests.
   `tests/integration_test.rs` uses `cargo_bin_cmd!` to spawn the compiled
   `gh-sync` binary. Process spawning is not supported under Miri.

5. **YAML parsing (`libyml` pointer arithmetic)** — ~64 tests.
   Tests in `gh-sync-manifest/src/manifest.rs` and
   `gh-sync-engine/src/mode/validate.rs` are skipped with
   `"libyml ptr_offset_from UB under Miri"`. The `libyml` C binding uses
   pointer arithmetic that triggers Miri's strict UB detection.

6. **Preserve-marker integration tests** — 9 tests.
   `gh-sync-engine/tests/preserve_markers.rs` tests that exercise file I/O
   and/or process spawning patterns.

### Statistics

| Metric                      | Count |
| --------------------------- | ----- |
| Total tests                 | 562   |
| Miri-compatible             | 454   |
| Miri-ignored (per-test)     | 108   |
| Miri-excluded (crate-level) | 0     |
