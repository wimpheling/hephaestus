# Durable agent mailboxes

An installed agent instance owns one durable mailbox. A mailbox is a
control-plane delivery primitive: it accepts bounded, provider-neutral work
and starts a normal agent run when that work is eligible. It is not an
application queue API, a gateway, or a protocol implementation. Released agent
code interprets the event and owns its application-level idempotency.

## Delivery contract

Acceptance means that PostgreSQL has durably committed the opaque body, its
integrity evidence, immutable envelope metadata, delivery row, and a wake-up
outbox record. It does not mean that the agent ran or that an upstream protocol
received an application response.

Each event has a mailbox-scoped producer and deduplication key. Repeating the
same key returns the original event rather than creating another logical
delivery. The generic envelope has a bounded method, route, selected headers,
content metadata, receive time, optional trace context, and opaque body ID.
The current limits are deliberately small:

- method: 16 bytes; route: 1,024 bytes;
- at most 32 selected headers, with names up to 64 bytes and values up to
  1,024 bytes;
- content metadata: 256 bytes; trace context: 512 bytes; and
- body: at most 1 MiB, stored as identity-encoded bytes with an exact SHA-256
  hash and byte length.

The control plane does not parse application payload schemas. Compression is
currently rejected at the acceptance boundary; a bounded, audited decompressor
policy is required before accepting compressed bodies.

Opaque body bytes are retained for 30 days. A worker-only retention pass may
then remove the bytes only after the delivery is terminal (`delivered`,
`dead_lettered`, or `cancelled`). The body ID, exact lengths, integrity hash,
accepted event, delivery history, and audit evidence remain immutable, so
deduplication and operator inspection never depend on retaining application
content indefinitely.

## Authority and lifecycle

The mailbox belongs to exactly one project-owned agent instance. It has no
independent application identity and a gateway is never an implicit producer.
Publishing, inspection, recovery, and consumption are checked through the
owning instance's authorization relation. PostgreSQL RLS applies those checks
to interactive access; workers use their dedicated role for durable dispatch
transitions.

An accepted event advances through these durable states:

```text
pending → eligible → leased → running → delivered
                                  │
                                  ├→ retryable → eligible
                                  ├→ denied
                                  ├→ dead_lettered
                                  └→ cancelled
```

`denied` records a stable diagnostic and may become eligible after a later
authorized recovery. `delivered`, `dead_lettered`, and `cancelled` are terminal.
The logical attempt count, next eligible time, disposition, and dead-letter
decision belong to PostgreSQL, never to the broker.

Eligibility rechecks the mailbox and instance lifecycle, open run gate, active
runnable revision, release state, and the exact authorization requirements.
Only then does dispatch select the current immutable instance revision and
create the durable run request. Queued work therefore does not silently bind a
release revision when it is accepted.

## PostgreSQL, outbox, and JetStream

PostgreSQL is the sole source of truth for accepted events, bodies,
deduplication, dispositions, attempts, retry timing, authorization evidence,
and recovery. An acceptance transaction creates the initial `pending` delivery
and a versioned wake command together. Subsequent eligibility, retry, cancel,
and recovery transitions likewise write an identifier-only command to the
transactional outbox in their authoritative transaction.

JetStream transports these versioned commands:

- `heph.mailbox.v1.wake`
- `heph.mailbox.v1.dispatch`
- `heph.mailbox.v1.retry`
- `heph.mailbox.v1.cancel`
- `heph.mailbox.v1.recover`

The command contains only a stable operation ID and mailbox event ID. Bodies,
credentials, capability bearers, and mutable attempt state never enter NATS.
The outbox ID is the `Nats-Msg-Id`; a short-lived PostgreSQL claim prevents two
publishers from settling the same outbox record. A consumer acknowledges only
after its PostgreSQL transition has committed or is proven idempotently
complete. JetStream redelivery is therefore a transport diagnostic, not a
logical retry counter.

## Runs, state, retries, and recovery

Dispatch creates one normal run through the existing run orchestrator. Its
normal lifecycle and authorization checks remain the VM boundary; the mailbox
does not provision or destroy a guest itself. Before provisioning, the mailbox
attempt records the exact run authorization snapshot and, when state is
required, the state-volume ID, lease ID, and lease fencing token. A monotonic
dispatch sequence per instance preserves the ordering evidence for stateful
work.

For a mailbox-dispatched run, the existing read-only guest control mount also
contains `mailbox-event.json` and `mailbox-body`. The JSON is the bounded,
generic envelope and opaque IDs; the body is a separate exact byte file. They
are materialized from PostgreSQL after durable claim, never placed in NATS,
logs, metrics, or runtime-authority handoffs.

An attempt is settled only after the run reaches durable `CleanedUp`, meaning
guest destruction and required lease cleanup are complete. A successful run is
`delivered`; a failed run schedules exponential backoff capped at one hour and
becomes `dead_lettered` after 100 logical attempts. State-access evidence is
honest rather than speculative: `no_state`, `completed_access`,
`failed_access`, or `uncertain_access` says nothing about which application
files changed.

Recovery first reconciles cleaned runs from their durable outcome. Other stale
leased or running deliveries become retryable and retain `uncertain_access`
when the platform cannot prove state access completed. A restarted supervisor
never assumes that an agent process, in-memory queue, or workflow stack
survived.

## Inspection and observability

The read-only inspection surface is intended to show an event's current
disposition, attempts, denial reason, selected revision, state volume, fenced
lease, dispatch sequence, state-access outcome, and next recovery action. It
must not reveal payload bytes, bearer material, or secret values.

The durable records provide the identifiers required for bounded traces:
mailbox, event, attempt, instance, revision, run, authorization snapshot,
volume, lease, outbox command, and operation. Trace fields must use those
opaque IDs only; routes, selected headers, trace context, payload metadata, and
broker error text should not become unbounded labels or sensitive log fields.

The operator metrics projection derives mailbox aggregates from PostgreSQL,
never broker delivery counters: acceptance-to-dispatch latency, queue depth,
active runs, retries, dead letters, denials, unpublished commands, and purged
payload counts. It contains no event, instance, producer, route, header, trace,
or payload identifiers. Delivery changes emit a redacted `AgentInstanceChanged`
product event; the existing instance watch reauthorizes every event before
delivery, and clients refresh the separately authorized mailbox inspection
projection after that wake.

## Operational verification

The ordinary unit suite covers domain validation and transport serialization.
Full mailbox evidence also needs real PostgreSQL, NATS JetStream, and libkrun:
concurrent duplicate acceptance, transaction rollback, RLS isolation,
publisher retry, acknowledgement loss, replay, worker crash, stale lease
recovery, and guest cleanup. Those environment-backed scenarios remain opt-in,
like the existing run-orchestration integration tests described in
[`run-orchestration.md`](run-orchestration.md).
