# Persistent gateway service runtime and development workflow

Owner: Astra orchestration (Luna bounded subtasks)

## Outcome

Let a gateway release run a normal long-lived web server inside a guest and
serve it through the existing shared Caddy deployment. This is the natural
development and production model for applications that already speak HTTP,
while retaining Hephaestus ownership of routing, authority, isolation,
lifecycle, and audit.

It is intentionally separate from MVP 03's stateless one-request gateway
handler. MVP 03 starts a fresh short-lived VM for one bounded request and
response; it needs a small stdin/stdout handler ABI, not a guest HTTP listener.

## Why this is a separate task

A persistent server changes fundamental runtime semantics:

- readiness and health checks;
- listener protocol and port ownership inside the guest;
- connection lifecycle, draining, cancellation, and process supervision;
- revision cutover and rollback while requests are in flight;
- scale, concurrency, idling, and crash recovery;
- local development parity, logs, and debugging.

Those concerns should not be hidden inside the synchronous, bounded MVP 03
contract or accidentally turn its microVM request path into a long-lived
service runtime.

## Initial direction

Use an explicitly declared service handler mode, distinct from `http.v1`
one-request handlers. Hephaestus starts the released command once, and the
command binds only a guest-private listener on a platform-provided transport.
The host-side gateway adapter forwards normalized requests from the shared
Caddy deployment over an authenticated private channel. Released code never
receives Caddy administration, a public listener, or unrestricted host
networking authority.

The exact guest transport (vsock, a private Unix socket bridge, or another
bounded mechanism) must be selected with its framing, flow-control, timeout,
and backpressure properties. It must not fall back to arbitrary guest TCP
exposure.

## Sequencing and coordination

Persistent services are the first implementation phase. Release-owned UI
surfaces depend on the service contract and begin only after this task's
implementation and focused acceptance evidence are complete. Reference chat
and MVP-06 work are explicitly out of scope until both phases are finished.

Astra owns architecture, decomposition, contract review, integration, and
commit coordination. Luna receives one bounded implementation, investigation,
or verification slice at a time, with explicit file ownership and acceptance
evidence. The exact guest transport and any other consequential cross-boundary
decisions remain pending main-thread review until their evidence is available.

## Implementation checklist

- [ ] **1. Service-mode release contract**
  - [ ] Inventory the existing stateless `http.v1` declaration and invocation
    path so service mode has an explicit compatibility boundary.
  - [ ] Define and validate the service-mode release configuration, command,
    immutable revision binding, entrypoint, resource limits, and compatibility
    or migration behavior without changing stateless handlers.
  - [ ] Persist and expose only the redacted service declaration data needed by
    authorized runtime and routing components.

- [ ] **2. Guest listener and private transport**
  - [ ] Compare the candidate bounded transports and record the selected
    framing, authentication, flow-control, timeout, and backpressure contract
    for main-thread review.
  - [ ] Implement guest listener ownership and authenticated host-to-guest
    forwarding on the reviewed transport; reject arbitrary guest TCP exposure
    and unauthorized host access.
  - [ ] Implement readiness, health, request, connection, response-size, and
    in-flight resource limits with deterministic timeout and cancellation
    semantics.
  - [ ] Define and implement request/response behavior for streaming,
    WebSockets, trailers, and unsupported protocol features, including the
    explicit bounded behavior where a feature is not supported.

- [ ] **3. Managed service lifecycle**
  - [ ] Supervise the released command as a long-lived guest process with
    startup, readiness, health, shutdown, cleanup, logs, and exit diagnostics.
  - [ ] Implement bounded concurrency, idling policy, crash detection,
    restart/recovery, and resource cleanup for service instances.
  - [ ] Implement revision cutover, request draining, rollback, and failure
    behavior while preserving the relationship among request, instance, and
    release revision.

- [ ] **4. Caddy and gateway integration**
  - [ ] Add host-side service forwarding behind the existing shared Caddy
    deployment without exposing Caddy administration or public host listeners
    to released code.
  - [ ] Implement route ownership, revision selection, load balancing or
    single-instance behavior, readiness gating, and deterministic unavailable
    responses.
  - [ ] Bind persistent connections and gateway-session authority to the
    authorized release, route, project, and revision context.
  - [ ] Preserve and regression-test the existing MVP-03 stateless one-request
    gateway path and its authority boundaries.

- [ ] **5. Security and authority boundaries**
  - [ ] Define and enforce host-header validation, client identity, TLS
    termination, internal endpoint protection, and release/project route
    isolation.
  - [ ] Test SSRF resistance, secret substitution boundaries, listener
    binding, cross-release access, cross-project access, and guest attempts to
    reach Caddy administration or host services.
  - [ ] Ensure logs and diagnostics redact request-only plaintext and secrets
    while retaining the audit and failure evidence needed for operations.

- [ ] **6. Development workflow**
  - [ ] Document local service execution, configuration, live-reload
    expectations, logs, debugger access, port inspection, and cleanup.
  - [ ] Provide a development path that exercises the same declaration,
    readiness, authority, and routing boundaries as production where practical,
    with explicit production-parity limits.

- [ ] **7. Focused implementation and acceptance tests**
  - [ ] Add focused tests for declaration validation, transport framing and
    limits, readiness/health, cancellation, lifecycle transitions, and
    stateless-mode compatibility.
  - [ ] Add real Caddy and guest-runtime integration coverage for forwarding,
    concurrent requests, cutover, draining, crash recovery, restart, rollback,
    and deterministic failure responses.
  - [ ] Add adversarial coverage for unauthorized listeners, Caddy/admin
    access, SSRF, host-header confusion, cross-release/project access,
    resource exhaustion, and secret leakage.

## Non-goals

This task does not replace MVP 03's bounded stateless invocation mode. It does
not grant guest Caddy admin access, public host ports, arbitrary listener
binding, or ambient project/network authority.

## Related work

- [MVP 03: Gateway HTTP routing and invocation](../done/mvp-03-event-ingress-and-caddy-routing.md)
- [MVP 03.1: Gateway principals and authority](../done/mvp-03.1-gateway-principals-and-authority.md)

## Verification gates

- [ ] Run `cargo fmt --all -- --check` after implementation changes.
- [ ] Run `cargo clippy --workspace --all-targets --all-features`.
- [ ] Run `cargo test --workspace --all-features`.
- [ ] Run `cargo doc --workspace --all-features --no-deps`.
- [ ] Run applicable Caddy, guest-runtime, browser, and focused integration
  checks with evidence from the current worktree.
- [ ] Run `cargo dev quality` after focused checks pass.
- [ ] Record the commands, results, test counts, runtime artifacts, and any
  deliberate follow-up tasks in Completion evidence before moving this task to
  `tasks/done/`.

## Completion evidence

- [ ] Record the reviewed service declaration and transport contract.
- [ ] Record real Caddy/guest-runtime forwarding, lifecycle, cutover,
  restart/recovery, and failure evidence.
- [ ] Record security/adversarial results and stateless MVP-03 regression
  evidence.
