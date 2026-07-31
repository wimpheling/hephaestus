# MVP 02: Durable agent mailboxes and stateful dispatch

Owner: unassigned

## Outcome

Give every installed agent instance a durable, generic mailbox and a
crash-recoverable dispatch path.

Accepted events are delivered at least once, deduplicated by stable platform
identities, and dispatched only while the instance lifecycle, run gate,
authorization, active revision, and state lease allow execution. Stateful
instances execute serially against one exclusively leased volume and restart
from durable application state rather than arbitrary process checkpoints.

## Locked decisions

| Area | Decision |
| --- | --- |
| Logical truth | PostgreSQL is authoritative for mailboxes, accepted events, deduplication, eligibility, attempts, dispositions, retry timing, run binding, and audit history. |
| Command transport | NATS JetStream carries versioned wake-up and dispatch commands. Stream state and delivery counters are not authoritative agent state. |
| Transactional bridge | Every command is written to a PostgreSQL transactional outbox with its authoritative state change and published with a stable `Nats-Msg-Id`. |
| Retry ownership | JetStream redelivery retries transport of one stable command. PostgreSQL owns logical attempt counts, retry eligibility, backoff, and dead-letter disposition. |
| Coordination | PostgreSQL compare-and-swap transitions, the instance run gate, and fenced volume leases provide concurrency control. |
| Execution | The existing run orchestrator and libkrun runtime own guest provisioning, execution, result handling, destruction, and cleanup. |
| Semantics | The platform provides durable at-least-once delivery. Released code owns application interpretation and application-level idempotency. |
| Envelope | Mailboxes carry bounded generic envelopes and opaque body references, never Telegram- or provider-specific schemas. |
| Payload storage | MVP event bodies are size-bounded and stored transactionally in PostgreSQL behind opaque body IDs. NATS commands never contain event bodies. |
| Ownership | A mailbox belongs to one project-owned agent instance. Publishing to it requires an explicit capability. |
| Stateful concurrency | An instance with one persistent state volume executes at most one normal stateful run or update hook at a time. |
| Run gate | The existing transactional instance run gate controls normal dispatch, update draining, recovery, and deferred work. |
| Revision binding | Queued events do not bind a revision until eligible dispatch. Dispatch binds the then-active immutable revision. |
| State evidence | Each stateful attempt records its exact state-volume ID, fenced lease identity, per-instance dispatch order, and terminal state-access outcome. It does not claim which bytes changed. |
| Sleep | A stopped agent loses process memory and reconstructs its loop from its release, immutable context, mailbox, and durable state. |
| Failure | Acknowledgement to an upstream producer means durable platform acceptance, not successful application processing. |

## Dependencies

- [`mvp-01-agent-principals-capabilities-and-runtime-authority.md`](mvp-01-agent-principals-capabilities-and-runtime-authority.md)
- [`reusable-agent-releases-and-instances.md`](../done/reusable-agent-releases-and-instances.md)
- [`define-own-the-loop-agent-platform.md`](define-own-the-loop-agent-platform.md)

## Non-goals

This task does not add public HTTP routing, protocol-specific gateways,
WebSockets, streaming services, arbitrary workflow-stack checkpointing,
stateless parallelism, state sharding, session volumes, schedules, or
interactive terminals.

It does not implement a message broker, consensus protocol, distributed log,
distributed lock, or general workflow engine.

## Implementation checklist

- [ ] **1. Define mailbox and delivery contracts**
  - [ ] **Add stable bounded domain types**
    - [ ] Add provider-neutral mailbox, event, delivery-attempt, disposition,
      body-reference, producer, and deduplication identifiers and values.
    - [ ] Define bounded methods, routes, selected headers, content metadata,
      receive timestamps, trace context, and opaque payload references.
    - [ ] Define pending, eligible, leased, running, delivered, retryable,
      denied, dead-lettered, and cancelled lifecycle transitions.
    - [ ] Define a monotonic per-instance dispatch sequence and bounded
      state-access outcomes for no state, completed access, failed access, and
      uncertain access.
    - [ ] Define deterministic identities for publish, dispatch, attempt,
      retry, cancellation, and dead-letter operations.
    - [ ] Add domain tests for validation, bounds, serialization, transitions,
      deterministic identities, and malformed envelopes.

- [ ] **2. Persist mailboxes and events transactionally**
  - [ ] **Add authoritative PostgreSQL records**
    - [ ] Add instance-owned mailboxes, immutable accepted events, delivery
      state, attempts, dispositions, payload references, and tombstone-safe
      provenance.
    - [ ] Enforce project and instance boundaries, immutable producer
      identity, unique deduplication keys in their declared scope, and bounded
      attempt state.
    - [ ] Write mailbox events and wake/dispatch outbox commands in the same
      transaction as acceptance or eligibility transitions.
    - [ ] Apply forced RLS and exact mailbox publish, consume, inspect, retry,
      and recover permissions.
    - [ ] Add real-PostgreSQL tests for concurrent duplicate publication,
      rollback, RLS, tenant isolation, visibility, and tombstone retention.
  - [ ] **Store bounded payloads safely**
    - [ ] Store each accepted body in PostgreSQL under an opaque body ID with
      its exact byte length and integrity hash in the acceptance transaction.
    - [ ] Enforce encoded and decoded size limits before commit and reject
      content-type confusion, malformed compression, and decompression bombs.
    - [ ] Put only the opaque mailbox-event and body IDs in commands, logs,
      traces, and NATS payloads.
    - [ ] Preserve payload bytes until every live delivery or audit retention
      requirement permits cleanup.

- [ ] **3. Dispatch eligible events**
  - [ ] **Integrate lifecycle and authorization**
    - [ ] At dispatch, recheck the mailbox, event, instance lifecycle, run
      gate, active revision, release access, capability binding, and target
      authorization in one durable transition.
    - [ ] Bind the event to the then-active immutable instance revision and
      authorization snapshot only when dispatch becomes eligible.
    - [ ] Mint runtime authority at dispatch and keep bearer material out of
      the durable event and command payload.
    - [ ] Persist a stable denial diagnostic when reauthorization fails rather
      than silently dropping accepted work.
    - [ ] Re-evaluate deferred events idempotently when the run gate reopens.
  - [ ] **Use JetStream as the command transport**
    - [ ] Publish versioned start, retry, cancel, and recovery commands with
      stable IDs and bounded exact provenance.
    - [ ] Write every command to the transactional outbox with the PostgreSQL
      state transition that makes it necessary.
    - [ ] Publish the outbox ID as `Nats-Msg-Id` and keep event bodies, bearer
      credentials, and mutable attempt state out of NATS.
    - [ ] Use durable consumers and acknowledge a command only after its
      corresponding PostgreSQL transition commits or is proven idempotently
      complete.
    - [ ] Make each consumer compare-and-swap authoritative PostgreSQL state so
      publisher retry and JetStream redelivery cannot create a second logical
      attempt or run.
    - [ ] Treat JetStream delivery counts as transport diagnostics only; never
      use them as mailbox attempt counts or dead-letter policy.
    - [ ] Add tests for database rollback, acknowledgement loss, publisher
      retry, duplicate delivery, consumer restart, stream replay, worker crash,
      and recovery at every authoritative transition.

- [ ] **4. Serialize stateful execution**
  - [ ] **Coordinate the instance run gate and volume lease**
    - [ ] Atomically prevent a second stateful normal run, update hook, or
      recovery action from becoming active for the same instance.
    - [ ] Use PostgreSQL lifecycle compare-and-swap, the existing run gate, and
      the existing fenced volume lease as the complete dispatch coordination
      mechanism.
    - [ ] Acquire and validate the instance volume's exclusive fenced lease
      before guest launch and release it only after guest destruction and
      provider cleanup.
    - [ ] Persist the exact state-volume ID, fenced lease ID and token, and
      per-instance dispatch sequence on the attempt before guest launch.
    - [ ] Prevent an old worker, expired lease holder, or duplicate command
      from completing or mutating the state of a newer attempt.
    - [ ] Integrate update draining so events accepted behind a closed gate
      remain durable and bind only after safe reopening.
    - [ ] Add concurrency tests for simultaneous ingress, repository triggers,
      retries, updates, pauses, cancellations, and stale lease holders.

- [ ] **5. Define retry, dead-letter, and recovery behavior**
  - [ ] **Handle outcomes honestly**
    - [ ] Define which provisioning, authorization, guest, protocol, timeout,
      and application outcomes are retryable, terminal, uncertain, or require
      operator recovery.
    - [ ] Persist the logical attempt count, next eligible time, bounded
      exponential backoff, attempt limit, and explicit dead-letter disposition
      in PostgreSQL without losing the original event.
    - [ ] Publish each newly eligible retry as a fresh stable outbox command
      for that logical transition.
    - [ ] Ensure application success is recorded only after the runtime result
      protocol and state cleanup reach their durable commit point.
    - [ ] Persist the terminal state-access outcome without inferring whether
      application-owned files changed or whether a failed run rolled them
      back.
    - [ ] Add authorized pause, resume, retry, cancel, and dead-letter
      inspection commands with structured audit.
  - [ ] **Reconcile crashes**
    - [ ] Reconcile abandoned dispatch claims, orphaned guests, stale volume
      leases, missing commands, and incomplete attempt transitions.
    - [ ] Ensure restart never assumes preservation of process memory or an
      in-memory workflow stack.
    - [ ] Add failure-injection tests before and after acceptance, dispatch
      commit, guest start, result commit, guest destruction, and lease release.

- [ ] **6. Add observability and operator inspection**
  - [ ] Trace mailbox, event, attempt, instance, revision, run, authorization
    snapshot, lease, and command identifiers.
  - [ ] Measure acceptance-to-dispatch latency, queue depth, active stateful
    runs, retries, dead letters, denials, and reconciliation outcomes.
  - [ ] Add read-only inspection for an event's current disposition, attempts,
    denial reason, bound revision, state volume, fenced lease, dispatch order,
    state-access outcome, and next recovery action.
  - [ ] Reauthorize live subscriptions before publishing mailbox or delivery
    updates.

- [ ] **7. Verify and document**
  - [ ] Document envelope limits, delivery semantics, revision-binding time,
    stateful serialization, retry policy, sleep/wake behavior, and recovery.
  - [ ] Document the PostgreSQL authority, transactional-outbox, JetStream
    transport, run-orchestrator, and fenced-volume-lease responsibility
    boundaries.
  - [ ] Run `cargo fmt --all -- --check`.
  - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
  - [ ] Run `cargo test --workspace --all-features`.
  - [ ] Run `cargo doc --workspace --all-features --no-deps`.
  - [ ] Run real-PostgreSQL, NATS, and real-libkrun mailbox scenarios.
  - [ ] Run `git diff --check`.

## Completion evidence

Record schema and subject versions, mailbox/event/run fixture IDs, delivery and
deduplication test counts, injected-crash evidence, state lease evidence,
serialized dispatch-order and terminal state-access evidence, latency
measurements, and deliberate follow-up tasks.
