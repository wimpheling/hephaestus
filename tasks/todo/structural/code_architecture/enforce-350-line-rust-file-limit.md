# Enforce a 350-line limit for Rust files

Owner: unassigned

## Outcome

Make the existing `ARCH-MAX-FILE-LENGTH` architecture rule a hard failure for
every hand-maintained Rust source file over 350 physical lines. Remove the
current layer-specific Rust ceilings above 350, split the existing violations,
and run the rule through `cargo dev check architecture` and `cargo dev quality`.
Do not introduce another rule ID or a Dylint dependency.

## Dependency and current state

- Start implementation after the [workspace topology migration](../../../in-progress/structural/code_architecture/clarify-workspace-crate-topology-and-extension-boundaries.md)
  completes. That task explicitly defers activating this rule until paths are
  stable. Recount the post-migration tree before splitting files or enabling the
  check; the pre-migration measurement below is a planning baseline, not an
  allowlist.
- The topology task's non-goal against a universal 300/350-line limit applies
  to that migration's scope. This separate task deliberately establishes the
  350-line Rust limit after the move; do not retain larger Rust thresholds on
  that basis.
- `ARCH-MAX-FILE-LENGTH` is already listed in the architecture registry and
  `architecture.toml`, but it is absent from `enabled_rules` and the current
  checker has no file-length scan. Its index entry remains migration-gated.
  Implement and activate this existing rule rather than creating a duplicate.
- The current `[maximum_file_lines]` values include Rust layer limits of 500 or
  700 lines. Every Rust source category must have an effective limit of exactly
  350; no Rust layer may retain a larger value or fall back to one. Leave
  unrelated Phoenix and UI thresholds unchanged.

## Locked policy

- Scan repository-owned Rust files under `crates/` and `examples/`, including
  library and binary modules, `build.rs`, unit and integration tests, and
  example code. Include untracked source files in a working checkout so a new
  file cannot bypass the check before it is committed.
- Count physical source lines. Blank lines, comments, test-only modules, and
  embedded Rust string contents count. A file with 350 lines passes; 351 lines
  fails. A final newline does not add an extra line.
- Exclude only the generated Rust output subtree owned by the `rpc-proto`
  package, at its post-migration path; `RPC-GENERATED-FILES-CLEAN` already
  checks that output against its generator. Exclude only the vendored third-
  party Rust subtree at
  `examples/cooking/cooking-gateway/vendor/`. Do not add a generic exclusion
  for directories named `generated` or `vendor`, generated-looking comments,
  tests, or large fixtures. Cargo `target/` build output is outside the source
  scan and is not repository Rust source.
- Do not preserve the migration backlog as warnings, a baseline file, or a
  new-code-only rule. The rule becomes a hard failure only after every
  hand-maintained violation is removed.
- Split a large file along cohesive module or test responsibilities, preserving
  public paths and behavior. Do not waive a file because it is inconvenient to
  split. There are no migration exceptions. If a future exception is necessary,
  first define a reviewed exact-file scope for this file-level rule; the
  current exception format requires an item or line. Require a rationale,
  owner, and expiry or tracking task for that one file.

## Pre-migration baseline

Measured from tracked `.rs` files on 2026-09-23 with physical line counts. Of
1,219 tracked Rust files, the measurement excluded 98 generated files under
`crates/rpc-proto/src/generated/` and 687 third-party files under
`examples/cooking/cooking-gateway/vendor/`. The remaining 434 hand-maintained
files comprise 417 under `crates/` and 17 under `examples/`. Of those, 213
exceed 350 lines: 200 under `crates/` and 13 under `examples/`.

These counts describe the current tree before the topology migration. Recompute
the baseline at implementation time and update this section if the inventory or
count changes; do not carry individual files forward as grandfathered entries.

## Implementation checklist

- [ ] After the topology migration, inventory the full checkout under `crates/`
  and `examples/`; record each over-limit path and line count, and confirm the
  generated and vendored exclusions are still exact.
- [ ] Split every hand-maintained file above 350 lines into cohesive Rust
  modules or test files. Preserve public APIs, test coverage, and runtime
  behavior; add re-exports only where needed to preserve existing paths.
- [ ] Implement the scan in the existing architecture checker. Apply a single
  effective 350-line ceiling to all Rust source files regardless of Cargo
  layer, including example packages without Hephaestus layer metadata.
- [ ] Add focused checker fixtures for 350-line acceptance and 351-line
  rejection, plus counting of blank/comment/test lines, nested source paths,
  untracked Rust files, and the exact generated/vendor exclusions. Diagnostics
  must name the path, observed line count, 350-line limit, and remediation to
  split the file.
- [ ] Set all Rust-specific configured thresholds to 350, add
  `ARCH-MAX-FILE-LENGTH` to `enabled_rules`, and update its `ARCHITECTURE.md`
  index entry from migration-gated warning to hard-enabled lint with its scope,
  command, and remediation. Keep the existing rule ID and exception registry.
- [ ] Confirm there are zero over-limit hand-maintained Rust files and zero
  migration exceptions before treating the rule as active.
- [ ] Run focused checker fixtures, `cargo dev check architecture`, and
  `cargo dev quality`; keep the workspace Rust, Clippy, and rustdoc lint
  baseline enabled.

## Acceptance criteria

- Every repository-owned Rust source under `crates/` and `examples/` is
  checked, with only the named generated RPC subtree and third-party vendor
  subtree excluded.
- A 350-line source passes and a 351-line source fails with a path, count,
  threshold, and split-remediation diagnostic. The check includes tests,
  build scripts, examples, and untracked sources.
- No Rust-specific architecture threshold exceeds 350, and
  `ARCH-MAX-FILE-LENGTH` is hard-enabled through `cargo dev check architecture`
  and `cargo dev quality`.
- All existing hand-maintained violations are split before activation; no
  grandfathered baseline or migration exception remains.
- The workspace topology task is complete before activation, and the final
  post-migration inventory and counts are recorded here.
