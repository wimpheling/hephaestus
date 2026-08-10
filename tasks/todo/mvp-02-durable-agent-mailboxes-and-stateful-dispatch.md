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
| Ownership | A mailbox belongs to one project-owned agent instance. A gateway is never an implicit mailbox owner; publishing to an agent mailbox requires an explicit capability. |
| Stateful concurrency | An instance with one persistent state volume executes at most one normal stateful run or update hook at a time. |
| Run gate | The existing transactional instance run gate controls normal dispatch, update draining, recovery, and deferred work. |
| Revision binding | Queued events do not bind a revision until eligible dispatch. Dispatch binds the then-active immutable revision. |
| State evidence | Each stateful attempt records its exact state-volume ID, fenced lease identity, per-instance dispatch order, and terminal state-access outcome. It does not claim which bytes changed. |
| Sleep | A stopped agent loses process memory and reconstructs its loop from its release, immutable context, mailbox, and durable state. |
| Failure | Acknowledgement to an upstream producer means durable platform acceptance, not successful application processing. |

## Dependencies

- [`mvp-01-agent-principals-capabilities-and-runtime-authority.md`](../done/mvp-01-agent-principals-capabilities-and-runtime-authority.md)
- [`reusable-agent-releases-and-instances.md`](../done/reusable-agent-releases-and-instances.md)
- [`define-own-the-loop-agent-platform.md`](define-own-the-loop-agent-platform.md)

## Non-goals

This task does not add public HTTP routing, gateway HTTP invocation,
protocol-specific gateways,
WebSockets, streaming services, arbitrary workflow-stack checkpointing,
stateless parallelism, state sharding, session volumes, schedules, or
interactive terminals.

It does not implement a message broker, consensus protocol, distributed log,
distributed lock, or general workflow engine.

## Implementation status (2026-08-09)

The bounded domain contract, authoritative PostgreSQL records, transactional
outbox, identifier-only JetStream transport, daemon loops, operator controls,
redacted inspection UI, and bounded tracing are implemented. The real
`scripts/test-mailbox-postgres-nats.sh` harness passes against disposable
PostgreSQL 17 and NATS JetStream, covering concurrent deduplication, outbox
publication, NAK redelivery, durable-consumer recreation, one-run claiming,
recovery to retryable, RLS tenant isolation, and tombstone retention.

Dispatch-time authorization is rechecked before provisioning, runtime
authority is minted only for the claimed run, and denied claims retain a
stable redacted disposition. The real suite covers a reopened run gate with
concurrent claims, stale fenced-lease cleanup, duplicate in-flight starts,
pre-provision denials, guest acknowledgement failure, timeout, and restart
after lease release. Payload retention keeps opaque bytes for 30 days, then a
worker-only terminal-delivery cleanup purges bytes while retaining event/body
identity and integrity evidence. The opt-in
`HEPHAESTUS_APP_LIBKRUN_E2E=1 scripts/run-libkrun-integration.sh` journey now
starts the production daemon with PostgreSQL and JetStream, accepts a real
mailbox event, and verifies that a real libkrun guest consumes the sealed
control files before the fenced state lease and VM are cleaned up.

## Implementation checklist

- [x] **1. Define mailbox and delivery contracts**
  - [x] **Add stable bounded domain types**
    - [x] Add provider-neutral mailbox, event, delivery-attempt, disposition,
      body-reference, producer, and deduplication identifiers and values.
    - [x] Define bounded methods, routes, selected headers, content metadata,
      receive timestamps, trace context, and opaque payload references.
    - [x] Define pending, eligible, leased, running, delivered, retryable,
      denied, dead-lettered, and cancelled lifecycle transitions.
    - [x] Define a monotonic per-instance dispatch sequence and bounded
      state-access outcomes for no state, completed access, failed access, and
      uncertain access.
    - [x] Define deterministic identities for publish, dispatch, attempt,
      retry, cancellation, and dead-letter operations.
    - [x] Add domain tests for validation, bounds, serialization, transitions,
      deterministic identities, and malformed envelopes.

- [x] **2. Persist mailboxes and events transactionally**
  - [x] **Add authoritative PostgreSQL records**
    - [x] Add agent-instance-owned mailboxes, immutable accepted events, delivery
      state, attempts, dispositions, payload references, and tombstone-safe
      provenance.
    - [x] Enforce project and instance boundaries, immutable producer
      identity, unique deduplication keys in their declared scope, and bounded
      attempt state.
    - [x] Write mailbox events and wake/dispatch outbox commands in the same
      transaction as acceptance or eligibility transitions.
    - [x] Apply forced RLS and exact mailbox publish, consume, inspect, retry,
      and recover permissions.
    - [x] Add real-PostgreSQL tests for concurrent duplicate publication,
      rollback, RLS, tenant isolation, visibility, and tombstone retention.
  - [x] **Store bounded payloads safely**
    - [x] Store each accepted body in PostgreSQL under an opaque body ID with
      its exact byte length and integrity hash in the acceptance transaction.
    - [x] Enforce encoded and decoded size limits before commit and reject
      content-type confusion, malformed compression, and decompression bombs.
    - [x] Put only the opaque mailbox-event and body IDs in commands, logs,
      traces, and NATS payloads.
    - [x] Preserve payload bytes until every live delivery or audit retention
      requirement permits cleanup.

- [x] **3. Dispatch eligible events**
  - [x] **Integrate lifecycle and authorization**
    - [x] At dispatch, recheck the mailbox, event, instance lifecycle, run
      gate, active revision, release access, capability binding, and target
      authorization in one durable transition.
    - [x] Bind the event to the then-active immutable instance revision and
      authorization snapshot only when dispatch becomes eligible.
    - [x] Mint runtime authority at dispatch and keep bearer material out of
      the durable event and command payload.
    - [x] Persist a stable denial diagnostic when reauthorization fails rather
      than silently dropping accepted work.
    - [x] Re-evaluate deferred events idempotently when the run gate reopens.
  - [x] **Use JetStream as the command transport**
    - [x] Publish versioned start, retry, cancel, and recovery commands with
      stable IDs and bounded exact provenance.
    - [x] Write every command to the transactional outbox with the PostgreSQL
      state transition that makes it necessary.
    - [x] Publish the outbox ID as `Nats-Msg-Id` and keep event bodies, bearer
      credentials, and mutable attempt state out of NATS.
    - [x] Use durable consumers and acknowledge a command only after its
      corresponding PostgreSQL transition commits or is proven idempotently
      complete.
    - [x] Make each consumer compare-and-swap authoritative PostgreSQL state so
      publisher retry and JetStream redelivery cannot create a second logical
      attempt or run.
    - [x] Treat JetStream delivery counts as transport diagnostics only; never
      use them as mailbox attempt counts or dead-letter policy.
    - [x] Add tests for database rollback, acknowledgement loss, publisher
      retry, duplicate delivery, consumer restart, stream replay, worker crash,
      and recovery at every authoritative transition.

- [x] **4. Serialize stateful execution**
  - [x] **Coordinate the instance run gate and volume lease**
    - [x] Atomically prevent a second stateful normal run, update hook, or
      recovery action from becoming active for the same instance.
    - [x] Use PostgreSQL lifecycle compare-and-swap, the existing run gate, and
      the existing fenced volume lease as the complete dispatch coordination
      mechanism.
    - [x] Acquire and validate the instance volume's exclusive fenced lease
      before guest launch and release it only after guest destruction and
      provider cleanup.
    - [x] Persist the exact state-volume ID, fenced lease ID and token, and
      per-instance dispatch sequence on the attempt before guest launch.
    - [x] Prevent an old worker, expired lease holder, or duplicate command
      from completing or mutating the state of a newer attempt.
    - [x] Integrate update draining so events accepted behind a closed gate
      remain durable and bind only after safe reopening.
    - [x] Add concurrency tests for simultaneous ingress, repository triggers,
      retries, updates, pauses, cancellations, and stale lease holders.

- [x] **5. Define retry, dead-letter, and recovery behavior**
  - [x] **Handle outcomes honestly**
    - [x] Define which provisioning, authorization, guest, protocol, timeout,
      and application outcomes are retryable, terminal, uncertain, or require
      operator recovery.
    - [x] Persist the logical attempt count, next eligible time, bounded
      exponential backoff, attempt limit, and explicit dead-letter disposition
      in PostgreSQL without losing the original event.
    - [x] Publish each newly eligible retry as a fresh stable outbox command
      for that logical transition.
    - [x] Ensure application success is recorded only after the runtime result
      protocol and state cleanup reach their durable commit point.
    - [x] Persist the terminal state-access outcome without inferring whether
      application-owned files changed or whether a failed run rolled them
      back.
    - [x] Add authorized pause, resume, retry, cancel, and dead-letter
      inspection commands with structured audit.
  - [x] **Reconcile crashes**
    - [x] Reconcile abandoned dispatch claims, orphaned guests, stale volume
      leases, missing commands, and incomplete attempt transitions.
    - [x] Ensure restart never assumes preservation of process memory or an
      in-memory workflow stack.
    - [x] Add failure-injection tests before and after acceptance, dispatch
      commit, guest start, result commit, guest destruction, and lease release.

- [x] **6. Add observability and operator inspection**
  - [x] Trace mailbox, event, attempt, instance, revision, run, authorization
    snapshot, lease, and command identifiers.
  - [x] Measure acceptance-to-dispatch latency, queue depth, active stateful
    runs, retries, dead letters, denials, and reconciliation outcomes.
  - [x] Add read-only inspection for an event's current disposition, attempts,
    denial reason, bound revision, state volume, fenced lease, dispatch order,
    state-access outcome, and next recovery action.
  - [x] Expose bounded, authorized operator controls for pause, resume, retry,
    cancel, and dead-letter through the supported application RPC/UI boundary;
    retain the PostgreSQL audit record as the authoritative control evidence.
  - [x] Reauthorize live subscriptions before publishing mailbox or delivery
    updates.

- [x] **7. Verify and document**
  - [x] Document envelope limits, delivery semantics, revision-binding time,
    stateful serialization, retry policy, sleep/wake behavior, and recovery.
  - [x] Document the PostgreSQL authority, transactional-outbox, JetStream
    transport, run-orchestrator, and fenced-volume-lease responsibility
    boundaries.
  - [x] Run `cargo fmt --all -- --check`.
  - [x] Run `cargo clippy --workspace --all-targets --all-features`.
  - [x] Run `cargo test --workspace --all-features`.
  - [x] Run `cargo doc --workspace --all-features --no-deps`.
  - [x] Run real-PostgreSQL, NATS, and real-libkrun mailbox scenarios.
  - [x] Run `git diff --check`.

## Completion evidence

Record schema and subject versions, mailbox/event/run fixture IDs, delivery and
deduplication test counts, injected-crash evidence, state lease evidence,
serialized dispatch-order and terminal state-access evidence, latency
measurements, and deliberate follow-up tasks.
