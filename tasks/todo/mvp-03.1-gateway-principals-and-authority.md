# MVP 03.1: Gateway principals and authority

Owner: unassigned

## Outcome

Implement the durable gateway resource and authority slice required to prove
MVP 01 against a concrete non-agent workload. A repository may declare a
gateway alongside its agents; Hephaestus persists its identity and immutable
revision, treats it as a durable authorization principal, and issues an
auditable, short-lived runtime session for a gateway invocation.

This is deliberately **not** public ingress. It neither configures Caddy nor
accepts Internet traffic nor starts a gateway handler VM. MVP 03 consumes these
records and authority APIs when it adds the shared-Caddy provider and the
bounded synchronous HTTP dispatcher.

```text
repository gateway declaration
  → gateway + immutable revision
  → exact capability bindings and user grants
  → invocation authorization snapshot
  → short-lived gateway runtime session
  → live checks, RLS, and audit
```

## Why this is separate

MVP 01 establishes a generic workload-principal and runtime-authority model.
Its acceptance criteria require gateway relations and invocation provenance,
but MVP 03 currently owns the gateway resource model. Implementing those
pieces ad hoc inside MVP 01 would create a second, incompatible gateway model;
implementing all of MVP 03 just to test authority would couple the foundation
to Caddy and HTTP delivery.

MVP 03.1 creates the single authoritative gateway model and proves the common
authority path. It gives MVP 01 its concrete gateway evidence and leaves edge
delivery entirely to MVP 03.

## Locked decisions

| Area | Decision |
| --- | --- |
| Declaration | Gateway declarations are repository configuration entries at the same level as agents. This slice validates stable names, a versioned handler-contract identifier, typed parameters, exposure mode, and bounded route intent, but does not make routes live. |
| Identity | A gateway is a project-owned durable workload principal, never an agent-instance subtype and never a Caddy configuration object. Its immutable revision is independently addressable and historically retained. |
| Authority | Gateway configuration, revision installation, inspection, enable/disable, removal, and invocation use explicit capability relations. Managing a gateway does not imply Caddy administration, repository access, state-volume access, mailbox publication, or secret access. |
| Invocation record | An invocation is an authoritative, auditable intent and runtime-session binding. It contains no inbound HTTP body, headers, provider credentials, route URL, or handler response. The actual HTTP request/response record belongs to MVP 03. |
| Runtime session | A successful authorized invocation creates a normal generic runtime session for the exact gateway revision. Its snapshot is the sole ceiling for privileged Hephaestus calls; live authorization and RLS still apply. |
| Route lifecycle | Route declarations, normalized route identity, and enabled lifecycle may be persisted for future reconciliation, but this task has no listener, URL output, provider request translation, dispatcher, or route reachability. |
| Secrets | Gateway revisions retain typed secret-slot contracts. Real webhook/provider values are not bound, substituted, or delivered here; MVP 04 and MVP 03 own brokered-placeholder use at the edge. |

## Dependencies

- [`mvp-01-agent-principals-capabilities-and-runtime-authority.md`](../done/mvp-01-agent-principals-capabilities-and-runtime-authority.md)
- [`reusable-agent-releases-and-instances.md`](../done/reusable-agent-releases-and-instances.md)
- [`define-own-the-loop-agent-platform.md`](define-own-the-loop-agent-platform.md)

## Implementation checklist

- [ ] **1. Define gateway declarations, identity, and lifecycle**
  - [ ] Extend repository configuration with gateway declarations beside agent
    declarations, stable names, normalized configuration identity, supported
    handler-contract version, typed parameters, exposure mode, secret slots,
    and bounded route intent.
  - [ ] Define gateway installation, immutable revision, update, pause,
    enable/disable, failure, recovery, removal, and tombstone semantics.
  - [ ] Reject unsupported handler contracts, ambiguous route intent, duplicate
    gateway names, cross-project references, and configuration that requests
    runtime mounts or authority outside exact capability bindings.

- [ ] **2. Persist the authoritative gateway model**
  - [ ] Add PostgreSQL records for gateway, immutable gateway revision,
    declared route intent, lifecycle transitions, and tombstone provenance.
  - [ ] Make PostgreSQL the sole authority; any future Caddy configuration is
    derived, reconstructible state.
  - [ ] Add forced RLS and compare-and-swap lifecycle transitions with real
    PostgreSQL tests for tenant isolation, conflicts, stale revisions, and
    historical retention.

- [ ] **3. Add gateway authorization relations and bindings**
  - [ ] Extend the canonical OpenFGA/Mélange model with gateway and gateway
    revision relations for inspect, configure, execute/invoke, update, pause,
    recovery, and removal.
  - [ ] Bind gateway capability slots through the existing immutable revision
    binding model; require explicit grant authority for every selected resource.
  - [ ] Prove that gateway management grants no ambient Caddy, repository,
    state-volume, mailbox, secret, or cross-project authority.

- [ ] **4. Snapshot and audit gateway invocation authority**
  - [ ] Add an invocation command that resolves one exact enabled gateway
    revision, reauthorizes it, creates the immutable authorization snapshot,
    and issues a short-lived generic runtime session.
  - [ ] Persist requestor, gateway, revision, invocation, session, snapshot,
    binding IDs, authorization-model version, decision, outcome, and request
    correlation IDs without recording HTTP payloads or secret values.
  - [ ] Enforce the same credential lifecycle, live revocation, RLS, session
    expiry, cancellation, and recovery rules as agent-run sessions.
  - [ ] Add tests for denied invocation, revision replacement, disabled and
    removed gateways, cross-project access, stale/revoked authority, credential
    replay, and audit redaction.

- [ ] **5. Expose minimal safe management and inspection**
  - [ ] Add typed RPC and UI projections for gateway declarations, revisions,
    lifecycle, capability requirements/bindings, and redacted invocation
    history.
  - [ ] Reauthorize every read and live update; never expose bearer material,
    secret values, provider credentials, HTTP bodies, or future Caddy details.

- [ ] **6. Verify and hand off to MVP 03**
  - [ ] Document the division: MVP 03.1 owns gateway identity and authority;
    MVP 03 owns Caddy reconciliation, public URL output, HTTPS termination,
    request translation, VM invocation, and HTTP response relay.
  - [ ] Run formatting, Clippy, tests, rustdoc, real-PostgreSQL authorization
    and lifecycle tests, `git diff --check`, and `cargo dev quality`.
  - [ ] Update MVP 01 completion evidence with concrete gateway-principal and
    invocation-session fixtures.

## Non-goals

This task does not open listeners, expose a URL, configure Caddy, terminate
HTTPS, route requests, invoke a handler VM, relay an HTTP response, substitute
webhook secrets, accept asynchronous events, publish to a mailbox, or define
provider-specific gateway adapters. Those remain MVP 03 (and brokered secret
delivery from MVP 04).
