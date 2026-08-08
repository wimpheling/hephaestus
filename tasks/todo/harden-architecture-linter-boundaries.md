# Harden architecture and boundary linting

Add a small set of high-value repository checks that protect authorization,
pagination, event durability, cancellation, and sensitive-data boundaries.
These rules should remain narrow, actionable, and enforced by the existing
`cargo dev check architecture` and `cargo dev quality` commands.

## Priority

- [ ] **P0 — `DB-RLS-CONTEXT-REQUIRED`**
  Reject PostgreSQL adapter queries executed through an application-role pool
  unless the query runs inside a transaction that establishes the canonical
  actor context (`hephaestus.actor_id`, subject type, request ID, and
  occurrence/idempotency provenance). Permit explicitly documented worker and
  migration exceptions only at their existing boundaries.
- [ ] **P1 — `DB-PAGINATION-STABLE-ORDER`**
  Require every paginated SQL query to use a deterministic `ORDER BY` ending in
  a unique tie-breaker, and require its cursor token to encode the same sort
  key. Reject page implementations that can skip or duplicate rows under
  concurrent writes.
- [ ] **P1 — `EVT-MUTATION-CAPTURE-COMPLETE`**
  Require authoritative state mutations and their canonical product-event
  capture to commit in the same transaction. Flag mutation paths that update
  durable state without an event trigger/capture or that publish before the
  durable commit.
- [ ] **P1 — `RPC-DEADLINE-CANCELLATION-PROPAGATION`**
  Reject RPC handlers and adapter calls that perform database, network,
  process, or VM work without propagating the request deadline and
  cancellation signal. Allow bounded background jobs only with an explicit
  ownership annotation.
- [ ] **P2 — `SEC-PLAINTEXT-TAINT-FLOW`**
  Extend the existing secret sentinel and descriptor checks with a narrow
  taint model: sensitive request values must not flow into logs, generic
  errors, JSON serialization, durable events, metrics labels, or response
  builders. Diagnostics should report the boundary and source field without
  printing the value.

## Implementation checklist

- [ ] Add each rule to the architecture rule registry with owner, rationale,
  enforcement command, migration gate, and the narrowest justified exception
  format.
- [ ] Add valid and invalid fixtures for every rule, including nested module,
  transitive helper, and test-only cases where applicable.
- [ ] Run the checks against the complete workspace and remove or annotate all
  existing violations; do not weaken the workspace lint baseline.
- [ ] Document the intended remediation beside each diagnostic and in
  `ARCHITECTURE.md`.
- [ ] Add focused Rust/Phoenix integration coverage for the runtime behaviors
  that static analysis cannot prove, especially RLS context, event atomicity,
  cancellation, and sensitive-value non-disclosure.
- [ ] Include all rules in `cargo dev check architecture` and
  `cargo dev quality`, then record the final enabled and migration-gated rule
  counts.

## Acceptance criteria

- [ ] An adapter query without actor-context setup fails the architecture
  check with a file/line diagnostic and remediation text.
- [ ] Every paginated query has a stable unique ordering and matching cursor
  contract, with concurrent-write fixtures covering duplicates and gaps.
- [ ] State/event atomicity and publish-after-commit are verified by a focused
  failure-injection test.
- [ ] Cancellation and deadline propagation are verified through a bounded
  RPC-to-adapter integration test.
- [ ] Sensitive request values remain absent from logs, errors, events,
  metrics, and responses under both success and failure paths.
- [ ] `cargo dev quality` passes with the new rule families enabled.
