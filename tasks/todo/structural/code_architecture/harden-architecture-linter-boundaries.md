# Harden architecture and boundary linting

Owner: unassigned

## Outcome

Strengthen the repository checks for authorization context, stable database
pagination, durable event capture, request cancellation, and sensitive-data flow.
Run the checks through the existing `cargo dev check architecture` and
`cargo dev quality` commands. Keep the strict workspace Rust, Clippy, and
rustdoc lint baseline enabled; use only narrow, accountable exceptions.

The SQLx boundary is already enabled as
`DB-SQLX-ONLY-IN-POSTGRES-ADAPTERS`. Preserve and regression-check its existing
metadata-based rule; do not add a duplicate SQLx rule. Workspace placement and
the core-to-standard dependency direction belong to the sibling
[workspace topology task](../../../in-progress/structural/code_architecture/clarify-workspace-crate-topology-and-extension-boundaries.md),
which owns `ARCH-CORE-NO-STD-DEPENDENCIES`.

## Locked decisions

- Keep `DB-SQLX-ONLY-IN-POSTGRES-ADAPTERS` enabled. SQLx production capability
  belongs only to packages declaring `package.metadata.hephaestus.postgres_adapter
  = true` and a valid `database_context`; dev-only SQLx fixtures require their
  existing explicit metadata. Production packages with that declaration must
  also keep a `-postgres` package name. The name is a discoverability check,
  never sufficient by itself to authorize SQLx. Strengthen the existing rule's
  fixtures and diagnostics for this naming guard; physical package placement
  belongs to the topology task.
- Strengthen `EVT-STATE-AND-EVENT-COMMIT-ATOMICALLY` for mutation capture and
  publish-after-commit guarantees. Do not introduce a parallel
  `EVT-MUTATION-CAPTURE-COMPLETE` rule ID.
- Extend the existing sensitive request, output, logging, and formatting rule
  families with a narrow, documented source-to-sink scope. Do not add a broad
  duplicate `SEC-PLAINTEXT-TAINT-FLOW` ID. Static checks must report source and
  sink locations without printing sensitive values.
- Keep SQL query ordering and cursor agreement distinct from the existing
  descriptor-level `RPC-LIST-HAS-PAGINATION` rule: protobuf checks require the
  collection contract, while this task checks adapter query ordering, unique
  tie-breakers, and cursor sort keys.
- State each static rule's detectable syntax or metadata contract explicitly.
  Verify behavioral claims that static analysis cannot prove with focused
  runtime or integration tests.
- Do not weaken workspace lint settings or add broad, permanent exceptions.
  Every temporary exception must use the existing exact-scope format and name
  its rationale, owner, and expiry or tracking task.

## Priorities

- [ ] **P0 — `DB-RLS-CONTEXT-REQUIRED`**
  Require PostgreSQL adapter query paths using an application-role connection
  to enter through a transaction boundary that establishes the canonical actor
  context: actor ID, subject type, request ID, and occurrence/idempotency
  provenance. Statically reject query paths that bypass the scoped transaction
  API. Test that context is present within a transaction and cleared between
  pooled requests. Permit worker and migration paths only at their existing,
  explicit boundaries.
- [ ] **P1 — `DB-PAGINATION-STABLE-ORDER`**
  For paginated SQL paths, require deterministic ordering ending in a unique
  tie-breaker and a cursor that encodes the same sort keys. Statically check the
  declared query/cursor contract; use concurrent-write integration cases to
  prove pages do not skip or duplicate rows. Keep this separate from the
  descriptor-level pagination requirement.
- [ ] **P1 — strengthen `EVT-STATE-AND-EVENT-COMMIT-ATOMICALLY`**
  Extend the existing rule beyond checking for durable-capture schema objects:
  verify the statically visible mutation/capture boundary and reject product
  publication before commit or outside the designated outbox path. Use
  failure-injection tests to prove state mutation and event capture commit
  atomically and that publication happens only after durable commit.
- [ ] **P1 — `RPC-DEADLINE-CANCELLATION-PROPAGATION`**
  Require RPC-to-application and adapter paths that perform database, network,
  process, or VM work to carry the request deadline and cancellation signal.
  Statically check explicit context/token propagation at supported call
  boundaries. Permit bounded background work only when ownership and lifetime
  are explicit. Verify cancellation and deadline behavior with a focused
  RPC-to-adapter integration test.
- [ ] **P2 — extend sensitive-data boundary checks**
  Extend the existing request-annotation, output-field, logging, and
  unrestricted-format checks with a narrow, documented set of source-to-sink
  flows. Cover direct local bindings and supported conversions from sensitive
  request fields into logs, generic errors, JSON, durable events, metric labels,
  and response builders. Verify non-disclosure through success and failure
  runtime paths; static diagnostics must never include plaintext values.
- [ ] **Regression guard — existing SQLx boundary**
  Keep `DB-SQLX-ONLY-IN-POSTGRES-ADAPTERS` enabled and preserve direct,
  transitive, and dev-only fixture coverage. Add a fixture rejecting a
  production `postgres_adapter = true` package without the `-postgres` suffix,
  and one rejecting a `-postgres` package without adapter metadata. Report any
  missed SQLx path as a defect in that existing rule, not as a new rule family.

## Dependencies and ownership

- The [workspace topology task](../../../in-progress/structural/code_architecture/clarify-workspace-crate-topology-and-extension-boundaries.md)
  owns canonical manifest-directory prefixes, provider placement, facades, and
  `ARCH-CORE-NO-STD-DEPENDENCIES`, including its normal, build, and development
  dependency edges. Do not duplicate that implementation or its fixtures here.
- This task depends on the existing architecture registry, exception format,
  SQLx metadata rule, and `cargo dev check architecture` / `cargo dev quality`
  entry points. Register only genuinely new rule IDs; update the existing event
  and sensitive-data rule definitions when extending their checks.

## Implementation checklist

- [ ] For each priority, define the statically enforceable syntax/metadata
  boundary, diagnostic and remediation, and any runtime behavior the checker
  cannot prove.
- [ ] Add valid and invalid fixtures for each static change, including nested
  module/helper paths and test-only cases where relevant. Retain regression
  fixtures for the existing direct, transitive, and dev-only SQLx boundary.
- [ ] Add focused Rust/Phoenix integration coverage for RLS context isolation,
  concurrent pagination, event atomicity and publish timing, cancellation,
  and sensitive-value non-disclosure.
- [ ] Use exact, item-level or line-level exceptions only where a real boundary
  cannot be expressed otherwise; document owner, rationale, and expiry or
  tracking task.
- [ ] Update the architecture rule index with each new rule's owner, rationale,
  command, state, and remediation. For extensions, update the existing rule row
  without adding a duplicate ID.
- [ ] Run focused fixture and integration checks, then `cargo dev check
  architecture` and `cargo dev quality`; resolve violations without weakening
  the lint baseline.
- [ ] Document the final static-analysis limits and required runtime evidence
  beside the diagnostics and in `ARCHITECTURE.md`.

## Acceptance criteria

- [ ] An adapter query path that skips canonical application-role context fails
  with a file/item diagnostic and remediation; runtime tests prove context
  isolation across pooled requests.
- [ ] Every paginated SQL path has matching stable query and cursor keys, with
  concurrent-write tests covering duplicate and missing rows.
- [ ] The existing `EVT-STATE-AND-EVENT-COMMIT-ATOMICALLY` rule detects the
  supported static violations, and failure-injection tests prove atomic capture
  and publish-after-commit behavior.
- [ ] Request deadlines and cancellation reach supported adapter operations;
  integration tests prove cancellation behavior through RPC-to-adapter paths.
- [ ] Existing sensitive request/output/log/format rules cover the agreed
  narrow source-to-sink cases, and runtime tests prove sensitive values remain
  absent from logs, errors, events, metrics, and responses on success and
  failure.
- [ ] `DB-SQLX-ONLY-IN-POSTGRES-ADAPTERS` remains enabled and rejects direct or
  transitive production SQLx capability outside metadata-declared PostgreSQL
  adapters; production adapters also use `-postgres` package names, while
  dev-only fixtures require explicit metadata.
- [ ] No rule or exception weakens the strict Rust lint baseline, and
  `cargo dev quality` passes with the completed changes.
