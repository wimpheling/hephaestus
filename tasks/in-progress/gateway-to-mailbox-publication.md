# Gateway-to-mailbox publication

Owner: codex

## Outcome

Let a released synchronous HTTP gateway publish one bounded, generic event to
an explicitly bound durable agent mailbox during a request. The gateway keeps
its bounded HTTP acknowledgement contract while the target agent receives
durable at-least-once delivery through the existing mailbox dispatcher.

This closes the generic bridge needed by the MVP-05 cooking journey:

```text
public request -> released gateway -> bound agent mailbox -> released agent
```

The platform does not learn Telegram, cooking, user mapping, event schemas, or
application retry semantics.

## Locked decisions

| Area | Decision |
| --- | --- |
| Authority | A gateway has no ambient mailbox access. An immutable gateway revision may bind exactly declared mailbox-publication capability slots to explicit target mailboxes, subject to grant authority. |
| Scope | A binding names one target mailbox and one stable producer identity. A gateway cannot select another mailbox, project, or producer at runtime. |
| Guest interface | The released handler receives a bounded, authenticated host operation for publishing a generic mailbox envelope and body with a stable application-supplied deduplication key. It has no database, NATS, control-plane bearer, Caddy-administration, or general network access. |
| Delivery | The host accepts the event transactionally through the existing mailbox authority and outbox path. A successful gateway response acknowledges gateway acceptance only; it never claims that the target agent completed processing. |
| Idempotency | Repeating the same producer identity and deduplication key returns the original logical event outcome without creating a second delivery. Gateway handler retries own their choice of key. |
| Gateway contract | `http.v1` remains a bounded synchronous request/response contract. Publication is a bounded host call made while handling that request; there is no long-lived gateway service or asynchronous gateway execution mode. |
| Provenance | Record gateway invocation/revision, capability binding, mailbox, producer, accepted event, authorization snapshot, and final gateway response disposition. Never record event bodies, credentials, raw request bodies, or provider headers in logs, traces, or unscoped inspection views. |
| Revocation | Publish reauthorizes live gateway revision, binding, grant, mailbox, and target-instance state. Revocation blocks later calls and produces a redacted durable denial; in-flight settlement follows documented transactional semantics. |

## Dependencies

- [`mvp-01-agent-principals-capabilities-and-runtime-authority.md`](../done/mvp-01-agent-principals-capabilities-and-runtime-authority.md)
- [`mvp-02-durable-agent-mailboxes-and-stateful-dispatch.md`](../done/mvp-02-durable-agent-mailboxes-and-stateful-dispatch.md)
- [`mvp-03-event-ingress-and-caddy-routing.md`](../done/mvp-03-event-ingress-and-caddy-routing.md)

## Non-goals

This task does not add provider-specific webhooks, Telegram semantics,
application event schemas, direct gateway-to-agent RPC, a public mailbox API,
general outbound networking, long-lived gateway processes, WebSockets,
streaming, general scheduling, or cross-project mailbox publication.

## Implementation checklist

- [x] **1. Define the bounded capability and guest protocol**
  - [x] Add a generic mailbox-publication resource and operation to the capability and authorization model, including strict project, gateway-revision, mailbox, producer, body, header, route, and deduplication bounds.
  - [x] Define the immutable gateway-revision slot declaration and exact mailbox binding shape; reject undeclared slots, cross-project targets, duplicate/ambiguous bindings, and producer impersonation.
  - [x] Define a versioned guest-to-host publication protocol authenticated by the invocation runtime session; it accepts only one bound slot, a bounded generic envelope/body, and a bounded deduplication key.
  - [x] Specify response, timeout, cancellation, replay, and denial results without leaking target mailbox contents, authorization internals, or credentials.

- [x] **2. Persist and authorize gateway bindings**
  - [x] Persist gateway mailbox capability requirements, immutable revision bindings, grants, authorization snapshots, and tombstone-safe historical resolution.
  - [x] Reauthorize publication against the live gateway revision, exact binding, mailbox, target instance, and grant before accepting an event.
  - [x] Add RLS and authorization tests for missing grants, revoked bindings, cross-project targets, changed gateway revisions, disabled gateways, removed mailboxes, and unauthorized inspection.

- [x] **3. Implement the runtime bridge**
  - [x] Expose the bounded publication operation to an `http.v1` gateway handler without granting ordinary TCP, DNS, NATS, PostgreSQL, Caddy administration, repository access, state-volume access, or arbitrary control-plane calls.
  - [x] Route accepted publications through the existing authoritative mailbox acceptance and transactional-outbox path with the bound producer identity.
  - [x] Preserve exact correlation between gateway invocation, publication operation, mailbox event, delivery, and later agent run while keeping bodies redacted outside authorized application execution.
  - [x] Ensure guest crash, host crash, invocation retry, duplicate protocol frames, and NATS redelivery cannot create more than one logical mailbox event for one producer/deduplication key.

- [x] **4. Prove normal and denied behavior**
  - [x] Add a released fixture gateway that validates a bounded request, publishes to its one bound mailbox, and returns an acknowledgement only after the documented acceptance outcome.
  - [x] Prove concurrent distinct requests, duplicate requests, and gateway retries result in the expected durable events and target-agent dispatch behavior.
  - [x] Prove a gateway cannot publish to another mailbox, another project, an unbound slot, or with another producer identity; prove revocation during a request produces the documented outcome.
  - [x] Prove no event body, secret, runtime credential, raw provider request, or target state appears in guest-visible authority files, logs, traces, NATS, browser payloads, or inspection responses.

- [ ] **5. Inspect and verify**
  - [x] Add authorized inspection that follows one gateway request through invocation, exact binding, mailbox acceptance, delivery, dispatch, and final disposition, while denying unprivileged viewers.
  - [x] Add real PostgreSQL, NATS JetStream, Caddy, and libkrun integration coverage for publication, deduplication, crash recovery, revocation, and provenance.
  - [x] Run `cargo fmt --all -- --check`.
  - [x] Run `cargo clippy --workspace --all-targets --all-features`.
  - [x] Run `cargo test --workspace --all-features`.
  - [x] Run `cargo doc --workspace --all-features --no-deps`.
  - [ ] Run `cargo dev quality` and `git diff --check`.

## Completion evidence

Record gateway/revision/binding/grant/mailbox/event/delivery/run IDs, exact
authorization snapshots, deduplication and crash-recovery evidence, denial and
revocation outcomes, redaction/sentinel results, provenance screenshots, and
the exact verification commands and test counts.

## Implementation evidence (pending live journey)

- Source fixture: [`examples/cooking/cooking-gateway`](../../examples/cooking/cooking-gateway); it uses one
  `cooking_requests` slot and a stable `telegram-update-<update_id>`
  deduplication key, with no mailbox identifiers, broker/database access, or
  credentials in its source.
- Automated checks passed on 2026-09-03: `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets --all-features`,
  `cargo test --workspace --all-features`, `cargo doc --workspace
  --all-features --no-deps`, and `git diff --check`.
- `crates/gateway-postgres/tests/postgres.rs` supplies opt-in real-PostgreSQL
  coverage. `scripts/run-gateway-libkrun-e2e.sh` supplies its disposable
  PostgreSQL and JetStream URLs to those tests after the joined
  Caddy/libkrun/daemon proof. Together they cover concurrent distinct
  requests, retry deduplication, target dispatch, grant revocation, worker and
  application RLS, crash/redelivery recovery, and redacted provenance. That
  external-service command and `cargo dev quality` remain operator-run
  evidence and are intentionally not claimed here.
