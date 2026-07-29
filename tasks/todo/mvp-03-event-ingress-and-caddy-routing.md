# MVP 03: Event ingress and Caddy routing

Owner: unassigned

## Outcome

Expose bounded public HTTP event routes through Caddy and durably deliver their
requests to installed gateway code without giving the public parser project or
repository authority.

Caddy terminates TLS and forwards to an always-available Hephaestus ingress
service. Hephaestus owns authoritative route bindings, limits, durable generic
request acceptance, gateway activation, and audit. A separately authorized
gateway instance validates application protocol details and may publish only
to explicitly bound agent mailboxes.

The MVP uses the existing general Linux microVM runtime for released gateway
code. Selecting or optimizing a fast isolate runtime is deliberately deferred.

## Locked decisions

| Area | Decision |
| --- | --- |
| Edge | Caddy owns public TLS and HTTP handling but is neither the ingress source of truth nor an agent-aware control plane. |
| Authority | Ingress bindings and their audit history are authoritative PostgreSQL records. Caddy configuration is derived and reconstructible. |
| Contract | The MVP supports durable HTTP event bindings only. It does not support live service bindings. |
| Acknowledgement | Hephaestus acknowledges only after the bounded generic request is durable. Gateway or agent completion is asynchronous. |
| Isolation | A gateway is released code running as a separate principal with no project state volume, repository mount, Caddy admin access, or implicit target-agent authority. |
| Protocol ownership | Gateway code owns Telegram signatures, application tokens, user mapping, normalization, and other protocol semantics. |
| Delivery | A gateway may publish normalized events only to mailboxes named by its explicit capability bindings. |
| Runtime | General Linux microVMs are the MVP compatibility baseline. V8, WebAssembly, and Unikraft are later optimization candidates. |

## Dependencies

- [`mvp-01-agent-principals-capabilities-and-runtime-authority.md`](mvp-01-agent-principals-capabilities-and-runtime-authority.md)
- [`mvp-02-durable-agent-mailboxes-and-stateful-dispatch.md`](mvp-02-durable-agent-mailboxes-and-stateful-dispatch.md)
- [`define-own-the-loop-agent-platform.md`](define-own-the-loop-agent-platform.md)

## Non-goals

This task does not add WebSockets, streaming responses, preview servers,
long-lived service processes, synchronous agent completion, scale-to-zero VM
snapshots, a Telegram implementation in platform core, arbitrary Caddy admin
access, or a V8/WebAssembly/Unikraft runtime bakeoff.

## Implementation checklist

- [ ] **1. Specify event ingress and gateway contracts**
  - [ ] **Define the public edge contract**
    - [ ] Define trusted proxy headers, request IDs, methods, bounded routes,
      header allowlists, body limits, timeouts, rate limits, safe failure
      responses, and client-address handling.
    - [ ] Define durable acknowledgement, duplicate request, retry, response,
      and outbound-reference semantics without protocol-specific fields.
    - [ ] Define which TLS certificate and domain-ownership operations remain
      with Caddy and which lifecycle state Hephaestus records.
    - [ ] Document that streaming, WebSockets, and synchronous guest responses
      are unsupported by the MVP event contract.
  - [ ] **Define gateway identity and lifecycle**
    - [ ] Define released gateway installation, immutable revision,
      capability binding, update, pause, failure, recovery, and removal
      semantics using existing release primitives where possible.
    - [ ] Ensure gateway runtime policy cannot include repository mounts,
      project agent state, canonical credentials, or unrestricted mailbox
      publication.
    - [ ] Add stable diagnostics for unsupported gateway runtime contracts.

- [ ] **2. Persist authoritative ingress bindings**
  - [ ] **Add domain and PostgreSQL models**
    - [ ] Add stable ingress-binding, domain-binding, route, gateway target,
      derived-configuration revision, and reconciliation identifiers.
    - [ ] Persist domain, bounded route and methods, event contract, gateway
      revision, enabled state, creator, lifecycle, and tombstone history.
    - [ ] Reject overlapping active routes, invalid wildcard use,
      cross-project targets, unverified domains, stale revisions, and
      unauthorized binding changes.
    - [ ] Apply forced RLS and capability-checked inspect, create, update,
      enable, disable, and remove operations.
    - [ ] Add real-PostgreSQL tests for route conflict races, tenant
      boundaries, lifecycle CAS, RLS, and tombstone provenance.

- [ ] **3. Reconcile Caddy configuration**
  - [ ] **Keep the administration boundary private**
    - [ ] Add a trusted reconciler that converts authoritative active bindings
      into deterministic Caddy configuration without exposing the Caddy admin
      API to released code.
    - [ ] Apply derived configuration atomically and record the exact desired
      and observed configuration revisions.
    - [ ] Recover deterministically after Caddy, reconciler, or database
      restart and remove disabled or tombstoned routes safely.
    - [ ] Define certificate acquisition, renewal, domain verification
      failure, route activation, and rollback behavior.
    - [ ] Add reconciliation tests for duplicate commands, partial failure,
      stale observations, restart, and conflicting desired revisions.

- [ ] **4. Accept bounded requests durably**
  - [ ] **Implement the always-available ingress service**
    - [ ] Resolve the exact enabled binding without trusting
      producer-controlled forwarding headers or route metadata.
    - [ ] Enforce method, route, body, header, timeout, and rate limits before
      durable acceptance.
    - [ ] Store the generic request envelope and bounded body reference
      transactionally with the gateway mailbox publication.
    - [ ] Return a stable safe acknowledgement only after commit and a safe
      retryable error when durability is unavailable.
    - [ ] Prevent request smuggling, decompression bombs, path normalization
      mismatches, host confusion, and internal or metadata endpoint routing.
    - [ ] Add protocol-level and adversarial tests through Caddy into the
      ingress service.

- [ ] **5. Run gateway releases with narrow authority**
  - [ ] **Dispatch public events**
    - [ ] Start the exact released gateway revision through the normal
      microVM runtime and immutable release mount.
    - [ ] Provide the generic ingress envelope and no project repository or
      agent state mounts.
    - [ ] Mint a gateway-scoped runtime credential permitting only the bound
      secret/broker operations and publication to explicitly selected agent
      mailboxes.
    - [ ] Require gateway code to record accepted, rejected, normalized, and
      duplicate application events without placing protocol semantics in the
      core.
    - [ ] Add tests proving a malicious gateway cannot inspect another
      project, acquire repository capability, target an unbound mailbox,
      change its route, or call Caddy administration.

- [ ] **6. Handle cutover, failure, and recovery**
  - [ ] Define binding and gateway revision cutover so accepted requests
    retain the exact route and gateway revision provenance.
  - [ ] Define behavior for paused gateways, revoked capability bindings,
    unavailable guests, poison events, retry exhaustion, and disabled routes.
  - [ ] Reconcile orphaned derived routes, accepted-but-undispatched events,
    stale gateway leases, and incomplete revision activation.
  - [ ] Add failure-injection tests around ingress commit, acknowledgement,
    Caddy reconfiguration, gateway dispatch, normalization, and mailbox
    publication.

- [ ] **7. Add observability and management UI**
  - [ ] Trace domain, route, binding, Caddy revision, ingress event, gateway
    instance/revision, delivery attempt, target mailbox, and request IDs.
  - [ ] Measure accepted, rejected, limited, duplicate, queued, normalized,
    failed, and dead-lettered requests without high-cardinality public data.
  - [ ] Add authorized project UI for domain/route binding, gateway revision,
    lifecycle, recent ingress, denials, and recovery.
  - [ ] Reauthorize live route and ingress subscriptions and never expose
    request bodies to unauthorized viewers.

- [ ] **8. Verify and document**
  - [ ] Document the Caddy trust boundary, authoritative records, event
    contract, gateway restrictions, limits, cutover, and recovery procedures.
  - [ ] Run `cargo fmt --all -- --check`.
  - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
  - [ ] Run `cargo test --workspace --all-features`.
  - [ ] Run `cargo doc --workspace --all-features --no-deps`.
  - [ ] Run real-PostgreSQL, NATS, Caddy, and real-libkrun ingress scenarios.
  - [ ] Run `mix precommit` in `web/`.
  - [ ] Run the relevant Playwright browser scenario.
  - [ ] Run `git diff --check`.

## Completion evidence

Record schema and Caddy versions, authoritative and derived configuration
fixture IDs, route-conflict and restart evidence, ingress limits, gateway
isolation evidence, end-to-end latency, test counts, and deliberate follow-up
tasks.
