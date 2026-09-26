# Harden architecture and boundary linting

Owner: Codex session on `chore/code-architecture-tasks` (2026-09-25)

## Outcome

Strengthen the repository checks for authorization context, stable database
pagination, durable event capture, request cancellation, and sensitive-data flow.
Run the checks through the existing `cargo dev check architecture` and
`cargo dev quality` commands. Keep the strict workspace Rust, Clippy, and
rustdoc lint baseline enabled; use only narrow, accountable exceptions.

The SQLx boundary is already enabled as
`DB-SQLX-ONLY-IN-POSTGRES-ADAPTERS`. Preserve and regression-check its existing
metadata-based rule; do not add a duplicate SQLx rule. Workspace placement and
the core-to-standard dependency direction were completed in the
[workspace topology migration](https://github.com/wimpheling/hephaestus/pull/57),
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

- The completed [workspace topology migration](https://github.com/wimpheling/hephaestus/pull/57)
  owns canonical manifest-directory prefixes, provider placement, facades, and
  `ARCH-CORE-NO-STD-DEPENDENCIES`, including normal, build, and development
  dependency edges. Do not duplicate that implementation or its fixtures here.
- This task depends on the existing architecture registry, exception format,
  SQLx metadata rule, and `cargo dev check architecture` / `cargo dev quality`
  entry points. Register only genuinely new rule IDs; update the existing event
  and sensitive-data rule definitions when extending their checks.

## Current static contracts and proof boundaries

The repository already contains focused implementations for the static portions
of these boundaries. The contracts below describe what the checkers can actually
recognize and the evidence still required for behavior they cannot infer.

- **RLS context:** `DB-RLS-CONTEXT-REQUIRED` scans the six inventoried
  application-role pool bindings in five known adapter sources. It recognizes
  `begin_actor_transaction` and
  `begin_repeatable_read_actor_transaction`, tracks transaction locals, and
  requires the verified context helper to set actor, subject, request, and
  occurrence settings. Only six exact migration-backed security-definer
  resolver forms are allowlisted. Dynamic SQL, opaque helper internals, and
  unlisted pool fields are outside the proof. The real database context reuse
  evidence is `crates/heph-core/control-plane/postgres/tests/app_pool.rs`.
- **Pagination:** `DB-PAGINATION-STABLE-ORDER` reads package-local
  `pagination.toml` contracts. Each entry has a 1-based query index, exact
  `order` and `cursor_keys`, a uniform cursor operator, an explicit unique
  tie-breaker listed in `unique_keys`, and one supported cursor mode:
  `scalar`, `tuple`, `uuid_row_lookup`, or the pinned `stored_function` mode.
  The checker compares the declaration with literal SQL `ORDER BY` and cursor
  predicates and reports stale declarations. Representative concurrent-write
  coverage passed for UUID pages, snapshot pages, and the organization
  composite-key insertion path in
  `crates/heph-core/gateway/postgres/tests/service_targets/uuid_pages.rs`,
  `crates/heph-core/gateway/postgres/tests/service_log_reader/snapshot.rs`,
  and `crates/heph-core/control-plane/postgres/tests/organization_pagination.rs`.
  Other implementations still rely on the static contract until their runtime
  paths are exercised; each path does not require a duplicate test merely for
  the rule to remain enforceable.
- **Durable events:** `EVT-STATE-AND-EVENT-COMMIT-ATOMICALLY` checks the durable
  migration for the append function, capture functions/triggers, and product
  outbox trigger. Its Rust scan strips `#[cfg(test)]`, rejects literal direct
  writes to `application_events` or `product_event_outbox`, and accepts
  `append_application_event` only with a recognized transaction executor in the
  same Rust item. It cannot prove dynamic SQL, deployed trigger state, crash
  timing, or arbitrary transaction ownership. Existing rollback, visibility,
  and schema evidence is under `crates/heph-app/tests/event_durability/`.
- **RPC budget propagation:**
  `RPC-DEADLINE-CANCELLATION-PROPAGATION` scans production RPC files, resolves
  supported service-method delegations, and requires one
  `RequestBudget::from_transport`, an approved budget helper, and syntactically
  bounded detached-stream waits. It recognizes only listed helper names,
  argument positions, adapter/receipt await markers, and send/receive/sleep
  forms. It cannot prove arbitrary future taint, opaque helper behavior, or
  runtime cancellation. The opt-in transport evidence is
  `crates/heph-app/tests/artifact_deadline_cancellation.rs` with its
  `test-fixtures` implementation.
- **Sensitive flows:** the Rust source checker tracks eight known request field
  names through local bindings, assignments, request-rooted access, references,
  casts, and simple expressions. It checks supported tracing/log macros,
  response/event/payload/error calls, JSON/event macros, metric-label methods,
  and sensitive output structs; three opaque ID conversions terminate the plain
  text flow. The production UI installation RPC matrix passed success/failure
  non-disclosure coverage at `crates/heph-app/tests/ui_installation_rpc.rs`.
  No request-derived metric-label emission exists in current production paths,
  so the metric sink is guarded by static fixtures and a runtime metric
  assertion is currently inapplicable. Interprocedural and dynamic behavior
  remains outside the checker. Focused source fixtures are in
  `crates/heph-dev/src/checks/architecture/rust_architecture/tests.rs`.
- **SQLx metadata:** `DB-SQLX-ONLY-IN-POSTGRES-ADAPTERS` follows normal and
  build dependency paths transitively, checks dev-only SQLx separately, and
  requires boolean `postgres_adapter = true`, a lowercase nonempty
  `database_context`, and a `-postgres` package name. A suffix alone grants no
  capability. Metadata regression fixtures are in
  `crates/heph-dev/src/checks/architecture/db_architecture_tests/metadata.rs`.

The static diagnostics identify paths/items and remediation without printing
sensitive values. Runtime claims below are recorded only for the PostgreSQL,
event, pagination, and sensitive-flow runs that have passed; RPC cancellation
and full-quality behavior remain unchecked.

The current disposable-service evidence passed for two app-pool context tests,
eleven event-durability tests, one organization composite-pagination test, and
one production UI-installation RPC success/failure matrix. These are the
runtime references for the RLS, event, representative concurrent-pagination,
and sensitive-flow portions above. RPC cancellation remains pending its real
run, and no metric-label runtime assertion is required while production emits
no request-derived metric labels.

## Implementation checklist

- [x] For each priority, define the statically enforceable syntax/metadata
  boundary, diagnostic and remediation, and any runtime behavior the checker
  cannot prove.
- [x] Add valid and invalid fixtures for each static change, including nested
  module/helper paths and test-only cases where relevant. Retain regression
  fixtures for the existing direct, transitive, and dev-only SQLx boundary.
- [x] Add focused Rust/Phoenix integration coverage for RLS context isolation,
  representative concurrent pagination, event atomicity and publish timing,
  and UI-installation sensitive-value success/failure non-disclosure. The
  disposable-service evidence is recorded above; RPC cancellation remains
  pending its real integration run.
- [x] Use exact, item-level or line-level exceptions only where a real boundary
  cannot be expressed otherwise; no new hardening exception was needed or
  added.
- [x] Update the architecture rule index with each new rule's owner, rationale,
  command, state, and remediation. For extensions, update the existing rule row
  without adding a duplicate ID.
- [x] Run focused fixture and available disposable-service integration checks,
  then `cargo dev check architecture`; resolve violations without weakening the
  lint baseline. The static, RLS, representative pagination, event, and UI
  installation sensitive-flow checks passed.
- [ ] Run `cargo dev quality` and the RPC cancellation integration check; these
  full-quality and cancellation gates remain pending.
- [x] Document the final static-analysis limits and required runtime evidence
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
