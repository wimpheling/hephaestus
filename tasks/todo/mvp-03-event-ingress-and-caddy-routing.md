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

- [`mvp-01-agent-principals-capabilities-and-runtime-authority.md`](../done/mvp-01-agent-principals-capabilities-and-runtime-authority.md)
- [`mvp-03.1-gateway-principals-and-authority.md`](mvp-03.1-gateway-principals-and-authority.md)
- [`mvp-04-brokered-model-and-outbound-capabilities.md`](mvp-04-brokered-model-and-outbound-capabilities.md)
- [`define-own-the-loop-agent-platform.md`](define-own-the-loop-agent-platform.md)

## Non-goals

This task does not add durable asynchronous gateway delivery, WebSockets,
streaming responses, trailers, preview servers, long-lived service processes,
scale-to-zero VM snapshots, platform-specific Telegram semantics, arbitrary
Caddy admin access, custom domains, certificate lifecycle management, or a
V8/WebAssembly/Unikraft runtime bakeoff.

Persistent guest web servers and their local-development workflow are tracked
separately in [Persistent gateway service runtime and development workflow](persistent-gateway-service-runtime-and-development-workflow.md).

## Implementation status (2026-08-09)

Implemented foundations include repository `heph.gateways.toml` declarations
with required `agent_name`, immutable gateway/revision/route persistence,
gateway authorization relations, exact release-agent resolution, invocation
sessions, host-only inbound secret leases, the bounded dispatcher, private
HTTP VM ABI, daemon composition, and Caddy reconciliation.

Real proof covers a rootless libkrun private-HTTP exchange with the guest
network disabled; PostgreSQL RLS/lifecycle-CAS/outbox races; Caddy forwarding,
route updates, tombstone removal, and platform-route preservation; and the
authorized project management UI with reauthorizing product-event watches.
The joined `scripts/run-gateway-libkrun-e2e.sh` scenario proves Caddy through
the daemon dispatcher, persisted gateway authority, exact released artifact,
and real libkrun VM with host-only inbound brokered-header substitution. The
Caddy provider owns one explicitly marked gateway subroute in an
operator-supplied complete baseline rather than replacing platform routes.
Persistent guest web servers remain deliberately out of scope in the linked
follow-up.

## Implementation checklist

- [x] **1. Specify gateway declarations and the HTTP contract**
  - [x] **Define the public edge contract**
    - [x] Define the canonical HTTP method, path/query, header, body,
      trusted-metadata, and response representations; reject ambiguous path
      normalization and duplicate or forbidden headers.
    - [x] Define request/response size limits, startup and execution deadlines,
      rate limits, client-disconnect cancellation, safe timeout/failure
      responses, and response-header allowlists.
    - [x] Discard producer-controlled forwarding headers and supply trusted
      scheme, authority, client address, and request ID metadata.
    - [x] Document that streaming, trailers, upgrades, WebSockets, and
      long-lived connections are unsupported.
  - [x] **Define gateway identity and lifecycle**
    - [x] Extend repository configuration so gateway declarations are siblings
      of agent declarations. Define stable gateway names, one or more bounded
      listener/route requests, handler-contract versions, typed parameters,
      exact secret slots, exposure mode, limits, and normalized configuration
      identity. Return the resolved URL as provisioning output.
    - [x] Define released gateway installation, immutable revision,
      capability binding, update, pause, failure, recovery, and removal
      semantics using existing release primitives where possible.
    - [x] Define the host-side `GatewayProvider` contract for route
      reconciliation and provider request/response translation. Make
      `LocalCaddyGatewayProvider` the only MVP implementation.
    - [x] Ensure gateway runtime policy cannot include repository mounts,
      project agent state, canonical credentials, or unrestricted mailbox
      publication. Route provider/webhook credentials only through MVP 04
      brokered placeholder substitution; never bootstrap them into the VM.
    - [x] Add stable diagnostics for unsupported gateway runtime contracts.

- [x] **2. Persist authoritative gateway routes**
  - [x] **Add PostgreSQL models**
    - [x] Add stable gateway, gateway-revision, route, gateway target,
      derived-configuration revision, and reconciliation identifiers.
    - [x] Persist bounded route and methods, HTTP contract, gateway
      revision, enabled state, creator, lifecycle, and tombstone history.
    - [x] Reject overlapping active routes, invalid wildcard use,
      cross-project targets, stale revisions, and
      unauthorized binding changes.
    - [x] Apply forced RLS and capability-checked inspect, create, update,
      enable, disable, and remove operations.
    - [x] Add real-PostgreSQL tests for route conflict races, tenant
      boundaries, lifecycle CAS, RLS, and tombstone provenance.

- [x] **3. Reconcile Caddy configuration**
  - [x] **Keep the administration boundary private**
    - [x] Add `LocalCaddyGatewayProvider`, a trusted reconciler that converts authoritative active bindings
      into deterministic `/gateway/` routes in the existing shared Caddy
      configuration without exposing the Caddy admin API to released code or
      creating a separate Caddy deployment.
    - [x] Apply derived configuration atomically and record the exact desired
      and observed configuration revisions.
    - [x] Recover deterministically after Caddy, reconciler, or database
      restart and remove disabled or tombstoned routes safely.
    - [x] Terminate HTTPS in Caddy and forward only normalized private HTTP to
      the dispatcher. Do not add certificate or domain lifecycle APIs.
    - [x] Add reconciliation tests for duplicate commands, partial failure,
      stale observations, restart, and conflicting desired revisions.

- [x] **4. Implement the GatewayDispatcher and synchronous invocation**
  - [x] **Invoke bounded HTTP handlers**
    - [x] Bind the dispatcher route resolver and invocation recorder to the
      persisted enabled gateway revision, authorization snapshot, and runtime
      session; a provider trait or unit-only recorder alone is insufficient.
    - [x] Extend runtime authority with a durable, short-lived gateway
      invocation session and immutable snapshot. Do not synthesize a normal
      agent run merely to reuse run-shaped persistence.
    - [x] Define and implement the bounded private-HTTP VM handler transport
      used after dispatch. The existing VM lifecycle/provisioning interface is
      not itself an HTTP request/response protocol.
    - [x] Bind route resolution and invocation lifecycle recording to the
      authoritative enabled PostgreSQL revision. Resolution starts from the
      normalized request path, selects the longest exact path segment, and
      rechecks the active revision/lifecycle while atomically accepting the
      invocation; persisted evidence contains only IDs, correlation, and
      terminal outcome.
    - [x] Extend that persisted bridge with the M03.1 gateway authorization
      snapshot and generic runtime-session issuance/acknowledgement handoff.
      The existing generic session tables are run-shaped, so this requires the
      deliberate gateway-session persistence extension rather than fabricating
      a run or weakening the session ceiling.
    - [x] Resolve the exact enabled gateway route without trusting
      producer-controlled forwarding headers or route metadata.
    - [x] Enforce method, route, body, header, timeout, and rate limits before
      VM launch and create an auditable per-invocation runtime session.
    - [x] Start the exact released gateway revision with its immutable release
      mount, invoke its HTTP handler over private HTTP, and relay only its
      bounded canonical response through the provider adapter.
    - [x] Define deterministic behavior for startup failure, guest failure,
      timeout, cancellation, paused gateways, revoked bindings, and retries.
    - [x] Prevent request smuggling, decompression bombs, path normalization
      mismatches, host confusion, and internal or metadata endpoint routing.
    - [x] Add protocol-level and adversarial tests through Caddy into the
      GatewayDispatcher.

- [x] **5. Run gateway releases with narrow authority**
  - [x] **Restrict gateway authority**
    - [x] Provide the normalized HTTP request and no project repository or
      agent state mounts.
    - [x] Mint a gateway-scoped runtime credential permitting only exact bound
      operations and publication to explicitly selected agent mailboxes. Permit
      brokered inbound-header and outbound HTTPS placeholder substitution only
      under its exact secret, route, and destination bindings.
    - [x] Compare an inbound secret header in constant time and rewrite only a
      valid value to its brokered placeholder before VM delivery. Leave an
      invalid value non-matching or reject it without revealing secret-match
      details.
    - [x] Keep application protocol semantics, placeholder-based protocol-secret
      validation, and response codes in gateway code without placing them in
      platform core.
    - [x] Add tests proving a malicious gateway cannot inspect another
      project, acquire repository capability, target an unbound mailbox,
      change its route, call Caddy administration, or obtain a real inbound
      secret while it can validate the authorized placeholder.

- [x] **6. Handle cutover, failure, and recovery**
  - [x] Define route and gateway revision cutover, in-flight request draining,
    cancellation, and exact provenance across retries and failures.
  - [x] Define behavior for paused gateways, revoked capability bindings,
    unavailable guests, timeout/retry exhaustion, and disabled routes.
  - [x] Reconcile orphaned derived routes, stale gateway leases, and incomplete
    revision activation.
  - [x] Add failure-injection tests around Caddy reconfiguration, gateway
    startup, request forwarding, response forwarding, timeout, and cleanup.

- [x] **7. Add observability and management UI**
  - [x] Trace route, binding, Caddy revision, gateway
    instance/revision, runtime session, provider translation, and request IDs.
  - [x] Measure accepted, rejected, limited, cancelled, timed out, failed, and
    completed requests without high-cardinality public data.
  - [x] Add authorized project UI and supported RPC controls for gateway
    installation, route binding, gateway revision, lifecycle, recent ingress,
    denials, and recovery.
  - [x] Reauthorize live route and ingress subscriptions and never expose
    request bodies to unauthorized viewers.

- [x] **8. Verify and document**
  - [x] Document the shared-Caddy boundary, canonical HTTP contract, provider
    adapter, typed parameters/brokered placeholders, URL output, TLS non-goal, gateway
    restrictions, limits, cutover, and recovery.
  - [x] Run `cargo fmt --all -- --check`.
  - [x] Run `cargo clippy --workspace --all-targets --all-features`.
  - [x] Run `cargo test --workspace --all-features`.
  - [x] Run `cargo doc --workspace --all-features --no-deps`.
  - [x] Run real-PostgreSQL, Caddy, and real-libkrun scenarios covering HTTP
    forwarding, status/header/body propagation, timeout, cancellation, route
    isolation, restart, and authorization denial.
  - [x] Run `mix precommit` in `web/`.
  - [x] Run the relevant Playwright browser scenario.
  - [x] Run `git diff --check`.

## Completion evidence

Record schema and Caddy versions, authoritative and derived configuration
fixture IDs, normalized HTTP contract fixtures, route-conflict and restart
evidence, invocation limits, response-propagation and cancellation evidence,
gateway isolation evidence, test counts, and deliberate follow-up tasks.
