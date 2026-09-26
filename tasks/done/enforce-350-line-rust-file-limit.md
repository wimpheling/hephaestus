# Enforce a 350-line limit for Rust files

Owner: Codex session on `chore/code-architecture-tasks` (2026-09-25)

## Outcome

Make the existing `ARCH-MAX-FILE-LENGTH` architecture rule a hard failure for
every hand-maintained Rust source file over 350 physical lines. Remove the
current layer-specific Rust ceilings above 350, split the existing violations,
and run the rule through `cargo dev check architecture` and `cargo dev quality`.
Do not introduce another rule ID or a Dylint dependency.

## Dependency and current state

- The workspace topology migration completed in [PR #57](https://github.com/wimpheling/hephaestus/pull/57),
  with 90 reconciled packages and `cargo dev quality` passing. Its one-off task
  was deleted as requested. The historical post-migration working inventories
  below are starting measurements for this implementation, not an allowlist.
- The topology task's non-goal against a universal 300/350-line limit applies
  to that migration's scope. This separate task deliberately establishes the
  350-line Rust limit after the move; do not retain larger Rust thresholds on
  that basis.
- `ARCH-MAX-FILE-LENGTH` is implemented in the existing architecture checker
  and is now hard-enabled in `architecture.toml`. Its index entry is active;
  this task activates the existing rule rather than creating a duplicate.
- The pre-activation `[maximum_file_lines]` configuration included Rust layer
  limits of 500 or 700 lines. Every Rust source category now has an effective
  limit of exactly 350; no Rust layer may retain a larger value or fall back to
  one. Unrelated Phoenix and UI thresholds remain unchanged.

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

These counts describe the pre-topology-migration tree and remain historical
context for the implementation. Do not carry individual files forward as
grandfathered entries.

## Historical post-migration starting snapshots

These 2026-09-25 measurements were working inventories retained as historical
starting points; they are not active baselines or exception lists:

- The `HEAD` snapshot on 2026-09-25 counted 1,265 Rust files under `crates/`
  and `examples/`, with 480 hand-maintained files. Of those, 216 exceeded 350
  lines, with 173,017 excess physical lines.
- The concurrent working-checkout snapshot on 2026-09-25 included untracked
  split files and counted 1,307 Rust files, 522 hand-maintained files, and 217
  over-limit files, with 165,115 excess physical lines.

These snapshots explain the starting inventory used during the split work. The
final inventory below is measured independently after all splits. Its adjacent
TSV is a zero-violation scope/count audit and intentionally contains no
historical per-path violation rows.

## Final implementation inventory

Recounted from the full checkout on 2026-09-26, including untracked Rust files:
2,836 Rust source files under `crates/` and `examples/`, with Cargo `target/`
build output omitted, and the exact 98 generated RPC and 687 Cooking vendor
files excluded. The remaining 2,051 hand-maintained files comprise 1,929 under
`crates/` and 122 under `examples/`. None exceed 350 lines and excess physical
lines total zero. The final scope and count inventory is recorded in
[`enforce-350-line-rust-file-limit.inventory.tsv`](enforce-350-line-rust-file-limit.inventory.tsv);
these totals are the final active-rule inventory, not a grandfathered baseline.

## Implementation checklist

- [x] After the topology migration, inventory the full checkout under `crates/`
  and `examples/`; record the historical aggregate counts in this task,
  confirm the generated and vendored exclusions, and publish the final
  zero-violation scope/count audit in the adjacent inventory artifact.
- [x] Split every hand-maintained file above 350 lines into cohesive Rust
  modules or test files. Preserve public APIs, test coverage, and runtime
  behavior; add re-exports only where needed to preserve existing paths.
- [x] Implement the scan in the existing architecture checker. Apply a single
  effective 350-line ceiling to all Rust source files regardless of Cargo
  layer, including example packages without Hephaestus layer metadata.
- [x] Add focused checker fixtures for 350-line acceptance and 351-line
  rejection, plus counting of blank/comment/test lines, nested source paths,
  untracked Rust files, and the exact generated/vendor exclusions. Diagnostics
  must name the path, observed line count, 350-line limit, and remediation to
  split the file.
- [x] Set all Rust-specific configured thresholds to 350, add
  `ARCH-MAX-FILE-LENGTH` to `enabled_rules`, and update its `ARCHITECTURE.md`
  index entry from migration-gated warning to hard-enabled lint with its scope,
  command, and remediation. Keep the existing rule ID and exception registry.
- [x] Confirm there are zero over-limit hand-maintained Rust files and zero
  migration exceptions before treating the rule as active.
- [x] Run focused checker fixtures, `cargo dev check architecture`, and
  `git diff --check`; keep the workspace Rust, Clippy, and rustdoc lint baseline
  enabled.
- [x] Run the full `cargo dev quality` workspace gate after concurrent source
  migrations settle. The clean gate passed with fresh disposable PostgreSQL
  17/NATS services and exit 0.

## Final validation

The focused `ARCH-MAX-FILE-LENGTH` checker fixtures pass, including exact-limit
acceptance, one-line-over rejection, nested source paths, untracked files,
physical-line counting, and the exact generated/vendor exclusions.
`cargo dev check architecture` passes with the rule hard-enabled, and
`git diff --check` passes for the activation changes. The clean full
`cargo dev quality` gate subsequently passed with fresh disposable PostgreSQL
17/NATS services and exit 0.

## Acceptance criteria

- [x] Every repository-owned Rust source under `crates/` and `examples/` is
  checked, with only the named generated RPC subtree and third-party vendor
  subtree excluded.
- [x] A 350-line source passes and a 351-line source fails with a path, count,
  threshold, and split-remediation diagnostic. The check includes tests,
  build scripts, examples, and untracked sources.
- [x] No Rust-specific architecture threshold exceeds 350, and
  `ARCH-MAX-FILE-LENGTH` is hard-enabled through `cargo dev check architecture`
  and `cargo dev quality`.
- [x] All existing hand-maintained violations are split before activation; no
  grandfathered baseline or migration exception remains.
- [x] The workspace topology task is complete before activation, and the final
  post-migration inventory and counts are recorded here.
