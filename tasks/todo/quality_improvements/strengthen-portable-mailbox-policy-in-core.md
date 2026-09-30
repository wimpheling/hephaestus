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
