# Strengthen portable mailbox policy in `heph-core`

Owner: unassigned

## Purpose

Make the mailbox lifecycle policy portable across storage providers while
keeping PostgreSQL responsible for durable concurrency and authorization
boundaries. The work should proceed in small stages so the existing PostgreSQL
and NATS delivery behavior remains the compatibility reference.

## Current evidence

- [`mailbox-domain`](../../../crates/heph-core/runtime/mailbox/domain/src/lib.rs)
  defines provider-neutral envelope validation, identities, operation IDs, and
  delivery types. [`state.rs`](../../../crates/heph-core/runtime/mailbox/domain/src/state.rs)
  defines the delivery state graph and `DeliveryState::can_transition_to`, but
  repository search currently finds that transition helper used only by the
  domain tests; production transitions are enforced elsewhere.
- The PostgreSQL adapter owns the eligibility update and dispatch preparation in
  [`dispatch_commands.rs`](../../../crates/heph-std/runtime/mailbox/postgres/src/dispatch_commands.rs)
  and [`dispatch_claim.rs`](../../../crates/heph-std/runtime/mailbox/postgres/src/dispatch_claim.rs),
  including live mailbox, instance, revision, release, attachment, and
  stateful-run checks.
- PostgreSQL also owns run settlement, retry and backoff classification, and
  the attempt limit in
  [`dispatch_lifecycle.rs`](../../../crates/heph-std/runtime/mailbox/postgres/src/dispatch_lifecycle.rs)
  and [`recovery.rs`](../../../crates/heph-std/runtime/mailbox/postgres/src/recovery.rs),
  plus operator-driven pause, resume, retry, cancellation, and dead-letter
  changes in [`operator.rs`](../../../crates/heph-std/runtime/mailbox/postgres/src/operator.rs).
- The dispatch-facing `MailboxDispatchStore` port is currently declared in
  [`mailbox-dispatch`](../../../crates/heph-std/runtime/mailbox/dispatch/src/nats/contract.rs),
  so another provider would need to rediscover mailbox policy from the std
  implementation and its SQL behavior.
- PostgreSQL-specific atomic locks, RLS, transactional outbox writes, and run
  creation must remain in the adapter. The schema and transaction boundaries
  in [`0031_durable_agent_mailboxes.sql`](../../../migrations/0031_durable_agent_mailboxes.sql)
  are part of the durability and authorization contract, not policy to move
  into the domain crate.

## Related patterns elsewhere

The same policy-placement question appears in several other lifecycle areas:

- Release lifecycle: core defines transitions in
  [`lifecycle.rs`](../../../crates/heph-core/forge/release/domain/src/lifecycle.rs),
  including `Draft -> Revoked`, while the PostgreSQL revoke operation updates
  only `Published` releases in
  [`release_publication.rs`](../../../crates/heph-std/forge/release/postgres/src/release_publication.rs).
- Secret status: core defines the status transition helper in
  [`policy.rs`](../../../crates/heph-core/auth/secret/domain/src/policy.rs),
  but production PostgreSQL revoke and purge paths make the decisions in SQL
  in [`lifecycle_commands.rs`](../../../crates/heph-std/secret/postgres/src/service/lifecycle_commands.rs).
  The core secret manager remains substantive runtime orchestration, so this is
  a targeted policy-ownership follow-up rather than a claim that the whole
  secret subsystem belongs in the adapter.
- Gateway service ownership: core owns the state model and provider-neutral
  port in [`service_ownership.rs`](../../../crates/heph-core/platform/gateway/domain/src/service_ownership.rs),
  while PostgreSQL owns transition and promotion decisions in
  [`transitions.rs`](../../../crates/heph-std/gateway/postgres/src/service_ownership/transitions.rs).

There are useful counterexamples to preserve: the run PostgreSQL adapter calls
the core transition rule under its row lock in
[`operations.rs`](../../../crates/heph-std/run/postgres/src/run_postgres/operations.rs),
UI installation lifecycle code does the same in
[`lifecycle.rs`](../../../crates/heph-std/forge/release/postgres/src/ui_installation/lifecycle.rs),
and image selection validation is already meaningful core policy in
[`validation_images.rs`](../../../crates/heph-core/platform/agent-config/src/validation_images.rs).

## Scope and design constraints

Keep this as an incremental policy extraction, not a broad mailbox rewrite.
Preserve the existing states, deterministic operation identities, opaque body
handling, authorization-denial classification, and provider behavior while
making decisions explicit and testable without PostgreSQL.

## Implementation checklist

- [ ] Inventory the current state transitions and decision inputs in the
      PostgreSQL dispatch, lifecycle, recovery, and operator paths. Record the
      existing attempt cap, retry delay cap, denial rules, and terminal-state
      behavior before changing code.
- [ ] Add pure `heph-core` settlement and retry policy functions. They should
      accept typed attempt/run facts and return an explicit disposition,
      retry scheduling decision, and any terminal denial or dead-letter reason;
      they must not perform I/O or depend on SQL types.
- [ ] Move the retry/backoff and run-outcome decision rules into those pure
      functions while leaving PostgreSQL to lock rows, update attempts and
      deliveries, enqueue committed commands, and commit the result in one
      transaction.
- [ ] Add a typed eligibility decision input and result in `heph-core` for the
      facts the adapter rechecks immediately before dispatch. Keep live row
      reads, advisory locking, RLS, compare-and-swap updates, and run creation
      in PostgreSQL.
- [ ] Define a core-facing mailbox use-case port only after the pure decisions
      are stable. Keep transport-specific NATS command publication and the
      PostgreSQL implementation boundary narrow enough that a second durable
      provider can consume the same policy.
- [ ] Reconcile domain and SQL validation bounds and encoding rules, including
      method, route, content metadata, headers, body limits, and accepted
      content encoding. Make any intentional provider-specific restriction
      explicit rather than allowing silent drift.
- [ ] Preserve and extend focused tests: core transition, eligibility,
      settlement, retry, cap, and backoff matrices; PostgreSQL acceptance,
      recovery, operator, RLS, locking, outbox, and run-creation tests; and
      NATS command idempotency and acknowledgement behavior.
- [ ] During follow-up review, compare the mailbox boundary with the related
      release, secret, and gateway patterns above, using the run, UI
      installation, and image-policy cases as references; track any
      cross-domain drift separately from this mailbox extraction.

## Acceptance criteria

- Settlement and retry outcomes are selected by deterministic, provider-neutral
  core functions with unit tests covering success, transient failure,
  authorization denial, attempt-limit dead lettering, retry timing, and
  interrupted-run recovery inputs.
- The PostgreSQL adapter consumes those decisions inside its existing
  transactions; row locks, RLS checks, transactional outbox effects, and
  atomic run creation remain PostgreSQL responsibilities.
- Eligibility is represented by a typed core decision contract, while the
  adapter still supplies current database facts and performs the final
  compare-and-swap under the instance concurrency guard.
- Core and PostgreSQL agree on all shared envelope and payload bounds, or the
  intentional difference is named, documented, and tested at the boundary.
- Existing PostgreSQL and NATS tests continue to prove deduplication,
  identifier-only commands, retry and recovery, stateful serialization,
  authorization denial, terminal retention, and cleanup ordering.
- The final change documents which policy moved into core and which SQL,
  transaction, and transport concerns deliberately stayed in their adapters.

## Completion evidence

Record the policy API and state/decision matrix, the provider boundary, the
validation-bound comparison, focused core tests, PostgreSQL and NATS regression
results, and any remaining provider-specific follow-up before moving this task
to `tasks/done/`.
