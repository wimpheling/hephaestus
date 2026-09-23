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

- [x] **5. Inspect and verify**
  - [x] Add authorized inspection that follows one gateway request through invocation, exact binding, mailbox acceptance, delivery, dispatch, and final disposition, while denying unprivileged viewers.
  - [x] Add real PostgreSQL, NATS JetStream, Caddy, and libkrun integration coverage for publication, deduplication, crash recovery, revocation, and provenance.
  - [x] Run `cargo fmt --all -- --check`.
  - [x] Run `cargo clippy --workspace --all-targets --all-features`.
  - [x] Run `cargo test --workspace --all-features`.
  - [x] Run `cargo doc --workspace --all-features --no-deps`.
  - [x] Run `cargo dev quality` with the pinned toolchain and `git diff --check` (recorded implementation-head quality evidence and this closeout's diff check).

## Completion evidence

The records below tie a released gateway request to a mailbox event, dispatch,
and agent run. The linked journey and integration suites cover deduplication,
crash recovery, denial, revocation, and redaction. The retained summaries omit
some per-record IDs and provenance screenshots; those limits are stated below
rather than inferred from aggregate results.

## Implementation evidence (journey and quality passed)

- Fixture: [`examples/cooking/cooking-gateway`](../../examples/cooking/cooking-gateway)
  declares one `cooking_requests` slot and uses the stable
  `telegram-update-<update_id>` deduplication key. The source contains no
  mailbox identifiers, broker/database access, or credentials.
- **Disposable local wrapper evidence, 2026-09-05:** the
  [Cooking README record](../../examples/cooking/README.md#recorded-verification-2026-09-05)
  reports one real daemon journey and four PostgreSQL/NATS regressions for
  `examples/cooking/run.sh`; `scripts/run-gateway-libkrun-e2e.sh` separately
  passed its ordinary gateway journey and four regressions. The README states
  the identified resources did not remain installed after fixture cleanup:

  | Evidence | Recorded identifier or outcome |
  | --- | --- |
  | Route | `2e1ce551-7985-4a3d-8231-1954f5af4847` |
  | Gateway revision | `b604910b-a208-4b21-a644-ec575b0706ab` |
  | Mailbox | `36441a8b-729a-4561-917e-4b9201e1711d` |
  | Event | `d8f69d9f-ff04-40c2-96a7-5de5c0b5c092` |
  | Run / authorization snapshot | `417922be-fb60-49d2-9962-37a32db83092` |
  | Cooking revision | `ddf12f57-967c-4e74-8769-f0f4e1defa0c` |
  | State lease | `e346d3ac-c8f3-4a89-8d27-7e4ea022f6e6`, fencing token `2` |
  | Dispatch / disposition | Sequence `1`, `delivered` |

- **Current GCP evidence, 2026-09-23:** the accepted [Cooking run 35799526119](https://github.com/wimpheling/hephaestus/actions/runs/35799526119)
  exercised workload head `4061545a21c6f3ce84f6e475f69e8d2819024efe`;
  all 15 required workload phases passed with zero failures. Its `golden-tests`
  phase runs the joined Cooking journey. The `database-tests` phase runs the
  unfiltered `gateway-postgres` integration test binary against disposable
  PostgreSQL and JetStream after the shared-Caddy/libkrun proof. The runbook
  records manifest `10726285171`, workload `10726165768`, controller
  `10726155828`, archive SHA-256
  `d002d5ec20bce85138e9e53713f82216f66a7d032d55d3ac2947cf600d985a02`, a
  clean scan of 38 files / 1,163,789 bytes, and independent VM, disk, and IP
  absence verification. See the [GCP Cooking evidence record](../../docs/gcp-cooking-ci.md#current-feature-and-evidence-status)
  and [MVP-05.1 acceptance record](../done/mvp-05.1-complete-cooking-acceptance.md).
- `cargo +1.88.0 dev quality` passed for the same implementation head; the
  recorded log is
  `/var/tmp/heph-cargo-dev-quality-4061545-20260923-final-1143909.log`.
  `git diff --check` also passed for this closeout.
- The local 2026-09-05 summary does not record binding, grant, or delivery-row
  IDs; it records only the delivered dispatch sequence, not a delivery UUID.
  Neither that summary nor the published current GCP record includes a
  provenance screenshot. The GCP artifact IDs above are run-level pointers and
  do not supply those per-record IDs. The local disposable run is distinct
  from the GCP run and does not record the current GCP workload head.
