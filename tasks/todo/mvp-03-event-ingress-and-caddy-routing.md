# MVP 03: Gateway HTTP routing and invocation

Owner: unassigned

## Outcome

Let a repository declare gateway workloads alongside agents. A gateway maps one
or more bounded public routes to a released, stateless HTTP request handler.
Hephaestus owns the declaration, authorization, revision selection, invocation,
audit, and route lifecycle; released handler code owns application protocol
semantics and its HTTP response.

The sole MVP edge implementation extends the existing shared Forge Caddy
deployment. Caddy terminates public HTTPS and forwards the `/gateway/` namespace
to a stable Hephaestus GatewayDispatcher. The dispatcher invokes the selected
gateway revision in a short-lived Linux microVM and relays its bounded HTTP
response. It does not provision a second gateway-specific server.

The MVP uses the existing general Linux microVM runtime for released gateway
code. Selecting or optimizing a fast isolate runtime is deliberately deferred.

## Locked decisions

| Area | Decision |
| --- | --- |
| Gateway model | Repository configuration declares gateways beside agents. A gateway is an independently versioned workload and durable principal, not an agent-instance subtype. Its declaration has typed parameters, exact secret bindings, bounded routes, and a handler contract. |
| Exposure and output | A gateway is public or reserves the future `heph_authenticated` exposure mode. Provisioning produces the resolved public URL; authenticated user-context forwarding is not implemented by this MVP. |
| Handler contract | The canonical gateway contract is bounded HTTP request to bounded HTTP response. The handler receives method, path/query, allowlisted headers, body, and trusted Hephaestus metadata; it returns status, allowlisted headers, and body. |
| Invocation | Synchronous request/response invocation is the sole MVP mode. Hephaestus invokes a stateless short-lived VM on the request path and relays its response. Durable asynchronous event delivery is a later optional gateway mode. |
| Edge | The existing shared Forge Caddy deployment owns public TLS and HTTP handling. Gateway routes occupy the reserved `/gateway/` namespace; platform routes such as the UI, API, and OCI registry remain host-owned routes outside gateway declarations. |
| Provider | A host-side `GatewayProvider` adapter reconciles desired routes and translates provider-specific requests and responses to/from the canonical HTTP contract. The sole MVP implementation is `LocalCaddyGatewayProvider`; Cloudflare and AWS adapters are deferred. |
| HTTPS | `LocalCaddyGatewayProvider` terminates HTTPS at the public edge. Gateway VMs use private HTTP invocation. The MVP provides no gateway certificate, domain-verification, issuance, renewal, or user-configurable TLS policy; routes use the preconfigured Caddy listener and hostname. |
| Trusted metadata | Provider-controlled forwarding headers are discarded. The adapter supplies trusted scheme, authority, client address, and request ID metadata to the dispatcher. |
| Limits | The MVP has bounded methods, paths, headers, request bodies, response headers/bodies, startup and execution time. It has no streaming, trailers, connection upgrades, WebSockets, or long-lived service processes. |
| Authority | Gateway route records and audit history are authoritative PostgreSQL records. Caddy configuration is derived and reconstructible. A gateway has no project state volume, repository mount, Caddy administration, mailbox, or agent authority unless an exact capability binding grants it. |
| Protocol ownership | Gateway code owns application protocol parsing, placeholder-based signature validation, user mapping, normalization, and response semantics. A real webhook secret is compared and rewritten to its brokered placeholder at the authorized route before VM delivery; provider credentials never enter the VM. |
| Runtime | General Linux microVMs are the MVP compatibility baseline. V8, WebAssembly, and Unikraft are later optimization candidates. |

## Dependencies

- [`mvp-01-agent-principals-capabilities-and-runtime-authority.md`](mvp-01-agent-principals-capabilities-and-runtime-authority.md)
- [`mvp-04-brokered-model-and-outbound-capabilities.md`](mvp-04-brokered-model-and-outbound-capabilities.md)
- [`define-own-the-loop-agent-platform.md`](define-own-the-loop-agent-platform.md)

## Non-goals

This task does not add durable asynchronous gateway delivery, WebSockets,
streaming responses, trailers, preview servers, long-lived service processes,
scale-to-zero VM snapshots, platform-specific Telegram semantics, arbitrary
Caddy admin access, custom domains, certificate lifecycle management, or a
V8/WebAssembly/Unikraft runtime bakeoff.

## Implementation checklist

- [ ] **1. Specify gateway declarations and the HTTP contract**
  - [ ] **Define the public edge contract**
    - [ ] Define the canonical HTTP method, path/query, header, body,
      trusted-metadata, and response representations; reject ambiguous path
      normalization and duplicate or forbidden headers.
    - [ ] Define request/response size limits, startup and execution deadlines,
      rate limits, client-disconnect cancellation, safe timeout/failure
      responses, and response-header allowlists.
    - [ ] Discard producer-controlled forwarding headers and supply trusted
      scheme, authority, client address, and request ID metadata.
    - [ ] Document that streaming, trailers, upgrades, WebSockets, and
      long-lived connections are unsupported.
  - [ ] **Define gateway identity and lifecycle**
    - [ ] Extend repository configuration so gateway declarations are siblings
      of agent declarations. Define stable gateway names, one or more bounded
      listener/route requests, handler-contract versions, typed parameters,
      exact secret slots, exposure mode, limits, and normalized configuration
      identity. Return the resolved URL as provisioning output.
    - [ ] Define released gateway installation, immutable revision,
      capability binding, update, pause, failure, recovery, and removal
      semantics using existing release primitives where possible.
    - [ ] Define the host-side `GatewayProvider` contract for route
      reconciliation and provider request/response translation. Make
      `LocalCaddyGatewayProvider` the only MVP implementation.
    - [ ] Ensure gateway runtime policy cannot include repository mounts,
      project agent state, canonical credentials, or unrestricted mailbox
      publication. Route provider/webhook credentials only through MVP 04
      brokered placeholder substitution; never bootstrap them into the VM.
    - [ ] Add stable diagnostics for unsupported gateway runtime contracts.

- [ ] **2. Persist authoritative gateway routes**
  - [ ] **Add PostgreSQL models**
    - [ ] Add stable gateway, gateway-revision, route, gateway target,
      derived-configuration revision, and reconciliation identifiers.
    - [ ] Persist bounded route and methods, HTTP contract, gateway
      revision, enabled state, creator, lifecycle, and tombstone history.
    - [ ] Reject overlapping active routes, invalid wildcard use,
      cross-project targets, stale revisions, and
      unauthorized binding changes.
    - [ ] Apply forced RLS and capability-checked inspect, create, update,
      enable, disable, and remove operations.
    - [ ] Add real-PostgreSQL tests for route conflict races, tenant
      boundaries, lifecycle CAS, RLS, and tombstone provenance.

- [ ] **3. Reconcile Caddy configuration**
  - [ ] **Keep the administration boundary private**
    - [ ] Add `LocalCaddyGatewayProvider`, a trusted reconciler that converts authoritative active bindings
      into deterministic `/gateway/` routes in the existing shared Caddy
      configuration without exposing the Caddy admin API to released code or
      creating a separate Caddy deployment.
    - [ ] Apply derived configuration atomically and record the exact desired
      and observed configuration revisions.
    - [ ] Recover deterministically after Caddy, reconciler, or database
      restart and remove disabled or tombstoned routes safely.
    - [ ] Terminate HTTPS in Caddy and forward only normalized private HTTP to
      the dispatcher. Do not add certificate or domain lifecycle APIs.
    - [ ] Add reconciliation tests for duplicate commands, partial failure,
      stale observations, restart, and conflicting desired revisions.

- [ ] **4. Implement the GatewayDispatcher and synchronous invocation**
  - [ ] **Invoke bounded HTTP handlers**
    - [ ] Resolve the exact enabled gateway route without trusting
      producer-controlled forwarding headers or route metadata.
    - [ ] Enforce method, route, body, header, timeout, and rate limits before
      VM launch and create an auditable per-invocation runtime session.
    - [ ] Start the exact released gateway revision with its immutable release
      mount, invoke its HTTP handler over private HTTP, and relay only its
      bounded canonical response through the provider adapter.
    - [ ] Define deterministic behavior for startup failure, guest failure,
      timeout, cancellation, paused gateways, revoked bindings, and retries.
    - [ ] Prevent request smuggling, decompression bombs, path normalization
      mismatches, host confusion, and internal or metadata endpoint routing.
    - [ ] Add protocol-level and adversarial tests through Caddy into the
      GatewayDispatcher.

- [ ] **5. Run gateway releases with narrow authority**
  - [ ] **Restrict gateway authority**
    - [ ] Provide the normalized HTTP request and no project repository or
      agent state mounts.
    - [ ] Mint a gateway-scoped runtime credential permitting only exact bound
      operations and publication to explicitly selected agent mailboxes. Permit
      brokered inbound-header and outbound HTTPS placeholder substitution only
      under its exact secret, route, and destination bindings.
    - [ ] Compare an inbound secret header in constant time and rewrite only a
      valid value to its brokered placeholder before VM delivery. Leave an
      invalid value non-matching or reject it without revealing secret-match
      details.
    - [ ] Keep application protocol semantics, placeholder-based protocol-secret
      validation, and response codes in gateway code without placing them in
      platform core.
    - [ ] Add tests proving a malicious gateway cannot inspect another
      project, acquire repository capability, target an unbound mailbox,
      change its route, call Caddy administration, or obtain a real inbound
      secret while it can validate the authorized placeholder.

- [ ] **6. Handle cutover, failure, and recovery**
  - [ ] Define route and gateway revision cutover, in-flight request draining,
    cancellation, and exact provenance across retries and failures.
  - [ ] Define behavior for paused gateways, revoked capability bindings,
    unavailable guests, timeout/retry exhaustion, and disabled routes.
  - [ ] Reconcile orphaned derived routes, stale gateway leases, and incomplete
    revision activation.
  - [ ] Add failure-injection tests around Caddy reconfiguration, gateway
    startup, request forwarding, response forwarding, timeout, and cleanup.

- [ ] **7. Add observability and management UI**
  - [ ] Trace route, binding, Caddy revision, gateway
    instance/revision, runtime session, provider translation, and request IDs.
  - [ ] Measure accepted, rejected, limited, cancelled, timed out, failed, and
    completed requests without high-cardinality public data.
  - [ ] Add authorized project UI for gateway route binding, gateway revision,
    lifecycle, recent ingress, denials, and recovery.
  - [ ] Reauthorize live route and ingress subscriptions and never expose
    request bodies to unauthorized viewers.

- [ ] **8. Verify and document**
  - [ ] Document the shared-Caddy boundary, canonical HTTP contract, provider
    adapter, typed parameters/brokered placeholders, URL output, TLS non-goal, gateway
    restrictions, limits, cutover, and recovery.
  - [ ] Run `cargo fmt --all -- --check`.
  - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
  - [ ] Run `cargo test --workspace --all-features`.
  - [ ] Run `cargo doc --workspace --all-features --no-deps`.
  - [ ] Run real-PostgreSQL, Caddy, and real-libkrun scenarios covering HTTP
    forwarding, status/header/body propagation, timeout, cancellation, route
    isolation, restart, and authorization denial.
  - [ ] Run `mix precommit` in `web/`.
  - [ ] Run the relevant Playwright browser scenario.
  - [ ] Run `git diff --check`.

## Completion evidence

Record schema and Caddy versions, authoritative and derived configuration
fixture IDs, normalized HTTP contract fixtures, route-conflict and restart
evidence, invocation limits, response-propagation and cancellation evidence,
gateway isolation evidence, test counts, and deliberate follow-up tasks.
