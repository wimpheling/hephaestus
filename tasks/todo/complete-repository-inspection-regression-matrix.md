# Complete repository inspection regression matrix

Owner: unassigned

## Outcome

Prove that the repository source and commit-diff inspection surface remains
safe, bounded, correctly authorized, and usable across real Git edge cases and
the browser. This completes the deferred regression matrix for the implemented
repository inspection feature.

## Locked decisions

| Area | Decision |
| --- | --- |
| Scope | This task adds regression coverage and narrowly scoped test seams only; it does not expand repository-inspection product behavior. |
| Git fixtures | Use real temporary Git repositories for merge, timeout, truncation, and object-visibility assertions. Do not mock Git output for these cases. |
| Authorization | Exercise the production repository authorization adapter and reauthorization on every commit-detail/progressive-diff read. |
| Browser proof | Use the existing Playwright E2E harness against the real Phoenix application and daemon. Do not substitute static HTML assertions for theme, responsive, or revocation behavior. |
| Safety | Fixtures containing hostile source must assert text-only rendering and must not introduce unsafe HTML rendering paths. |

## Dependencies

- Completed repository inspection implementation in
  [`inspect-repository-source-and-commit-diffs.md`](../done/inspect-repository-source-and-commit-diffs.md).
- The PostgreSQL authorization fixtures, repository browser RPC, and existing
  Playwright E2E harness.

## Non-goals

- New repository-browser features, editor behavior, or changes to repository
  authorization policy.
- Increasing production Git limits merely to make a test fixture convenient.

## Implementation checklist

- [ ] **1. Complete focused source-viewer coverage**
  - [ ] Add table-driven or property coverage for bounded language/path
    inference, including unknown extensions and hostile path-like input.
  - [ ] Test highlighter-unavailable/unsupported fallback, source line
    numbering, and both light and dark theme token selection.
  - [ ] Cover HTML/script-like text, malformed Unicode, and token delimiters
    as text-only DOM output.

- [ ] **2. Complete real-Git commit-diff coverage**
  - [ ] Add a real non-conflicting merge fixture and prove deterministic
    selected-parent behavior for each declared parent.
  - [ ] Extract the Git deadline behind a narrow test seam and prove a stalled
    command produces the bounded unavailable/truncated result without leaking
    tool output.
  - [ ] Add real fixtures for ordinary, large, malformed, and explicitly
    truncated changes in addition to the existing root, rename, deletion, and
    binary cases.

- [ ] **3. Prove authorization rechecks**
  - [ ] Add integration tests that revoke membership or repository access after
    an initial successful read, then verify commit detail and every subsequent
    progressive-diff read fail closed.
  - [ ] Prove hidden/unreachable commits, stale commit URLs, and a commit ID
    supplied against a different repository disclose neither metadata nor
    object bytes.

- [ ] **4. Add browser regression proof**
  - [ ] Extend the Playwright fixture with a source file and bounded commit
    diff, then test commit navigation, line anchors, and hostile source text.
  - [ ] Test syntax-highlighted content in both day and night themes, including
    the safe plain-text fallback.
  - [ ] Test narrow viewport rendering and revoked-access presentation without
    backend error text or unauthorized bytes.

- [ ] **5. Verify and document**
  - [ ] Record fixture topology, authorization mutation method, and screenshots
    or browser artifacts for both themes in the repository-inspection docs.
  - [ ] Run `cargo fmt --all -- --check`.
  - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
  - [ ] Run `cargo test --workspace --all-features`.
  - [ ] Run `cargo doc --workspace --all-features --no-deps`.
  - [ ] Run the focused Playwright regression suite and `cargo dev quality`.
  - [ ] Run `git diff --check`.

## Completion evidence

Record the exact focused test commands, real-Git fixture coverage, the
authorization mutation and denial assertions, browser artifacts for both
themes and narrow viewport, and the final repository-wide quality results.
