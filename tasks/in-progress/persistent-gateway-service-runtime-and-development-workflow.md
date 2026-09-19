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

The selected guest transport is the dedicated per-VM vsock bridge described
below. Its framing, flow-control, timeout, and backpressure properties are
part of the reviewed contract, and it must not fall back to arbitrary guest
TCP exposure.

## Reviewed transport and declaration slice

The reviewed transport is a dedicated per-VM guest-initiated vsock service
bridge, separate from the existing control and broker channels. The guest
bootstrap connects to a host-owned per-VM Unix socket on a fixed private vsock
port, authenticates each connection with its single-use challenge bound to
that per-VM socket, and carries one raw full-duplex byte stream per host-side
HTTP connection. The service command
binds only the declared `127.0.0.1` loopback port; the VM remains in
`NetworkMode::Disabled` and receives no `PortForward` or `passt` network.
Per-connection socket backpressure, bounded connection counts, handshake and
idle deadlines, and host-side drain provide the initial flow-control boundary.

The first implementation slice adds only the typed `http.service.v1` gateway
declaration: an exact loopback port from 1024 through 65535, a strict
origin-form readiness path, and a strict origin-form health path. Platform
code will choose timeout and concurrency policy. Service settings are required
for `http.service.v1` and forbidden for stateless `http.v1`; the optional
settings field is omitted from serialization when absent so existing stateless
normalized hashes remain unchanged. Database installation, VM launch, Caddy
forwarding, and runtime acceptance remain fail-closed until later slices.

The initial service edge accepts bounded HTTP only. WebSockets, upgrades,
trailers, and other unreviewed streaming behavior are rejected until a later
transport/runtime decision. Service readiness will require a successful HTTP
request to the declared readiness path; control-channel liveness is not service
readiness. Each request continues to use fresh gateway invocation/session and
secret leases, and the bridge challenge is never a request bearer. The
`http.service.v1` declaration does not accept the `http.v1` mailbox-publication
sideband; a separate service event contract is required before publication can
be enabled for a normal long-lived server.

## Sequencing and coordination

Persistent services are the first implementation phase. Release-owned UI
surfaces depend on the service contract and begin only after this task's
implementation and focused acceptance evidence are complete. Reference chat
and MVP-06 work are explicitly out of scope until both phases are finished.

Astra owns architecture, decomposition, contract review, integration, and
commit coordination. Luna receives one bounded implementation, investigation,
or verification slice at a time, with explicit file ownership and acceptance
evidence. The transport choice is reviewed above; remaining consequential
cross-boundary decisions stay bounded to their implementation slices and
require main-thread review before each integration step.

## Current implementation checkpoint

The VM contract, standalone host broker, and host-side provider/worker/FFI
wiring are implemented on the feature branch. The contract carries optional
validated private-service settings through the libkrun protocol while
preserving the stateless `http.v1` gateway-handler flag. The broker owns a
private Unix listener, single-use challenge binding, bounded reservations,
raw stream backpressure, and shutdown cleanup. Provider-owned brokers are
created before worker launch, the worker forwards an exact redacted
single-use challenge over the control channel, and libkrun maps the guest
service vsock port to the per-VM Unix socket without passt or IP networking.
The guest now validates service mode before launch, brings up only `lo`,
injects the declared loopback endpoint, and bridges bounded authenticated raw
connections with cancellation and half-close handling. Admission has separate
bounded pending and active slots, and one setup deadline covers scheduling,
admission, vsock connection, loopback connection, and handshake; forwarding
uses a separate bounded idle-I/O policy. The fourth real-VM attempt passed the
private transport, loopback HTTP readiness/health/identity, delayed-response
concurrency, capacity teardown and reopen, destroy, graceful shutdown, and
runtime/cgroup cleanup scenarios. HTTP readiness supervision, Caddy forwarding,
managed lifecycle recovery, and release UI remain unchecked.

The local materializer now gives each launch attempt a fresh UUID and seals its
immutable gateway/revision identity in a host-only metadata file under the
dedicated `gateway-services` namespace. Old and new instances can coexist;
stale staging cleanup is scoped to that namespace and must run only after the
supervisor establishes materialization quiescence. Recovery validates regular,
sealed metadata and mount trees, rejects symlinks and non-regular metadata,
and removes only an exact owned instance. Evidence: 11 focused
`run-runtime-local` tests, focused Clippy with `-D warnings`, formatting, and
`git diff --check` pass. Database activation, lifecycle supervision, and UI
integration remain pending.

The immutable service launch resolver now selects only a published
`http.service.v1` revision by the host-owned gateway/revision identity and
exact release-agent binding. It creates no invocation, runtime session, or
request bearer; the resulting `VmSpec` uses disabled networking, no runtime
authority, fixed platform transport limits, and identity labels. A recording
materializer test verifies exact release artifacts, parameters, mount
postconditions, cleanup on malformed mounts, and rejection of stateless,
mismatched, and revoked revisions. Real disposable PostgreSQL evidence is
retained in `/tmp/heph-service-launch-postgres-20260919-attempt2.log` with
`REAL_POSTGRES_CONNECTED_AND_MIGRATED=1`, fixture seeding, and resolver query
markers; the focused test passed 1 test in 1.31s. Supervisor activation and
application integration remain pending.

The gateway-edge HTTP adapter now provides one bounded HTTP/1 request/response
exchange over an already authenticated private service stream. It emits an
origin-form request with the trusted `Host`, computed `Content-Length`, and
`Connection: close`, while removing forwarding, proxy, hop-by-hop, and dynamic
`Connection`-nominated headers. Responses require explicit `Content-Length` or
decoded chunked framing; bodyful close-delimited responses, upgrades (including
non-101 `Upgrade` headers), trailers, ambiguous framing, and unsupported
streaming behavior are rejected. HEAD and 304 representation lengths are
preserved, while 204 responses omit `Content-Length`. The adapter applies one
exchange deadline, bounded body/header/path policies, cancellation-safe driver
ownership, and rejects pathological public policy values. Its wire-header bound
is an aggregate canonical-header limit; Hyper's HTTP/1 parser has an 8 KiB
minimum buffer, so the policy does not claim a raw-wire bound below that size.
Evidence: 12 focused `gateway-edge` service HTTP tests, including raw
`Content-Length` plus `Transfer-Encoding` rejection, unannounced trailer
rejection, upgrade rejection, and caller-cancellation stream closure; focused
Clippy, package checks, formatting, and diff checks pass. This is adapter-only
support: live service mode, readiness/health supervision, Caddy forwarding,
managed lifecycle, and release UI wiring remain unchecked.

The gateway-edge probe slice now provides synthetic readiness and health GETs
over a running VM's private service connection. It uses only the validated
declared probe path and trusted platform authority, with no caller headers,
secret substitution, invocation identity, or bearer. Probe responses are
bounded to 8 KiB bodies, 32 headers, and 8 KiB aggregate headers, and only 2xx
statuses pass. A checked deadline begins before private-stream acquisition;
the remaining duration is passed to the HTTP adapter and the original deadline
also wraps the complete exchange. Probe failures are redacted and do not stop
or destroy the VM. Evidence: 4 focused probe tests cover path/Host
canonicalization, non-2xx and oversized responses, control-bootstrap versus
HTTP-readiness separation, total open-plus-exchange timeout, and cancellation
closing the private stream. Instance ownership, durable admission, lifecycle
supervision, and Caddy integration remain pending.

- [x] Finish focused VM contract tests and workspace compatibility checks:
  `cargo test -p vm-trait`, `cargo test -p vm-libkrun --lib`,
  `cargo test -p vm-fake`, focused Clippy, `cargo check --workspace
  --all-targets --all-features`, `cargo fmt --all -- --check`, and
  `git diff --check` pass in this worktree.
- [x] Review and integrate the contract slice before starting bridge work;
  the reviewed contract is present in the committed VM schema checkpoint.
- [x] Implement and test the standalone host broker with real Unix sockets:
  authenticated raw bidirectional I/O, redacted challenges, wrong/unknown and
  replay rejection, pending cancellation and expiry, active-capacity bounds,
  shutdown of pending and active connections, pending-read wakeup, and
  incomplete-handshake cancellation. Evidence: `cargo test -p vm-libkrun
  --lib service_transport::tests` (8 passed) and focused Clippy pass.
- [x] Wire the host provider, worker command, shared handshake protocol, and
  fixed-vsock FFI mapping. Provider connection calls enforce the declared
  timeout across worker dispatch and authenticated broker connection; a
  bounded control semaphore keeps timed-out sends from interrupting a worker
  frame, and capacity acquisition fails fast so the declared connect timeout
  is one total operation deadline. Evidence: `cargo test -p vm-libkrun --lib`
  (72 passed),
  `cargo clippy -p vm-libkrun --all-targets --all-features`,
  `cargo check --workspace --all-targets --all-features`, formatting, and
  `git diff --check` pass; `cargo doc --workspace --all-features --no-deps`
  exited 0 with no warnings.
- [x] Verify provider-level cancellation and lifecycle ownership: blocked
  worker control sends remain bounded by the service semaphore, timed-out
  offers release broker capacity, and worker exit closes an active provider
  stream. Focused provider tests are included in the 72 passing lib tests.
- [x] Keep configured private-service VMs outside the one-shot wall-clock
  timeout; their lifetime ends through explicit stop/destroy or worker exit,
  while ordinary VMs retain the existing deadline. Evidence: `cargo test
  -p vm-libkrun --lib` (72 passed), focused ordinary/service wall-clock tests
  (2 passed), focused Clippy, and a mutation check where removing the service
  guard made the service-survival test fail.
- [x] Implement the guest-side bridge slice: validate service settings and
  service-only authority exclusions, enable guest loopback via ioctl, inject
  `HEPH_SERVICE_HOST`/`HEPH_SERVICE_PORT`, authenticate on the dedicated vsock,
  enforce bounded pending and active connection caps, forward both directions
  with half-close semantics, and close active connections on cancellation,
  control EOF, or child exit. One total setup deadline covers pending admission
  and connection setup. Evidence: `cargo test -p vm-libkrun --bin heph-init`
  (15 passed), focused guest-binary Clippy, the exact musl guest build, and
  formatting pass. `scripts/run-libkrun-integration.sh` passed once in
  `/tmp/heph-libkrun-integration-20260919-attempt4.log` (1 test, 7.26s),
  including private transport capacity teardown/reopen and cleanup. HTTP
  readiness supervision, Caddy forwarding, managed lifecycle recovery, and
  release UI remain unchecked.

## Implementation checklist

- [ ] **1. Service-mode release contract**
  - [x] Add typed `http.service.v1` declaration/config validation and focused
    tests while preserving stateless serialization; persistence and runtime
    acceptance remain unchecked.
    - Evidence: `cargo test -p gateway-domain -p agent-config`,
      `cargo clippy -p gateway-domain -p agent-config --all-targets
      --all-features`, `cargo fmt --all -- --check`, and `git diff --check`
      pass for this declaration slice.
  - [ ] Inventory the existing stateless `http.v1` declaration and invocation
    path so service mode has an explicit compatibility boundary.
  - [ ] Define and validate the service-mode release configuration, command,
    immutable revision binding, entrypoint, resource limits, and compatibility
    or migration behavior without changing stateless handlers.
  - [x] Persist typed service declaration fields through migration 0070 and
    gateway install, project, and clone flows; runtime activation and
    readiness remain unchecked.
    - Evidence: disposable Postgres focused persistence test 1 and complete
      gateway integration target 7 passed, with focused Clippy and formatting
      checks passing.

- [ ] **2. Guest listener and private transport**
  - [ ] Compare the candidate bounded transports and record the selected
    framing, authentication, flow-control, timeout, and backpressure contract
    for main-thread review.
  - [x] Implement guest loopback ownership and authenticated host-to-guest
    forwarding on the reviewed transport; reject arbitrary guest TCP exposure
    and unauthorized host access. The bridge has bounded pending and active
    slots and one total admission/setup deadline. Focused guest tests and the
    fourth real-VM transport acceptance pass; HTTP readiness supervision and
    lifecycle behavior remain unchecked.
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

### Host-mediated session foundation checkpoint

- [x] Add migration 0071 and durable session admission modes. `guest_handoff`
  retains the existing pending, bearer-handoff, and acknowledgement contract;
  `host_mediated` is active without a guest bearer or bootstrap acknowledgement,
  stores no credential verifier, preserves the exact authorization snapshot and
  identity, serializes concurrent retries by invocation, and rejects guest
  acknowledgement, mode mutation, credential mutation, invalid host lifecycle
  shapes, changed-identity retries, and retries after revocation.
- [x] Add `issue_gateway_service` with immutable `http.service.v1` versus
  `http.v1` contract checks, preserve ordinary guest issuance, and reject
  direct guest repository creation for service revisions. Service mailbox
  sideband, dispatch integration, secret completion wiring, readiness-gated
  activation, and long-lived lifecycle management remain pending.
- [x] Real disposable PostgreSQL evidence: with the exact
  `HEPHAESTUS_POSTGRES_TEST_URL` environment variable, the runtime-authority
  test printed `REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 migration=71` and passed
  1 test in 1.28s; the gateway PostgreSQL target passed 7 tests in 1.94s.
  Focused Clippy, formatting, and diff checks pass. Retained log:
  `/tmp/hephaestus-service-authority-postgres-20260919-real.log`.
- [x] Add migration 0072 terminal cleanup for accepted host-mediated
  invocations: completed, failed, and timed-out outcomes revoke the durable
  session and active secret leases atomically; revoke and expiry paths use the
  same invocation-to-session-to-lease lock order, and stateless completion
  remains unchanged. Host issuance locks and verifies the accepted invocation
  and exact gateway/revision before creating authority. The resolver also
  requires an accepted invocation. Real PostgreSQL coverage includes terminal
  cleanup, retries after completion, revoke/expiry lease cleanup, the
  concurrent race winner cases, and 129 expired host sessions across bounded
  expiry batches. The exact-environment run passed 1 test in 1.64s; retained
  log: `/tmp/hephaestus-service-authority-terminal-cleanup-real-batched.log`.
  Dispatch acceptance, completion/cancellation wiring, and a separate
  cancellation reaper remain pending.
- [x] Gateway acceptance now selects the immutable handler contract from the
  authoritative insert and routes `http.service.v1` to host-mediated issuance
  while preserving `http.v1` guest issuance. Service acceptance fails closed
  without a runtime issuer, while stateless no-issuer acceptance remains
  compatible. Invocation, live session, and lease setup use the same lock
  order, and issuance, lease, unknown-contract, and TTL failures terminally
  reject the invocation. The exact-environment PostgreSQL acceptance suite
  printed `REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 migration=72` and passed 4
  tests in 1.53s; evidence is retained at
  `/tmp/hephaestus-gateway-acceptance-real.log`. Live service execution,
  cancellation completion wiring, and recovery reaping remain pending.
- [x] Add a bounded recovery method for abandoned `http.service.v1`
  invocations. It processes at most 128 accepted rows per transaction with
  `FOR UPDATE SKIP LOCKED`, rechecks eligibility after locking, includes active
  sessions whose expiry has passed, and terminalizes through the existing
  invocation completion path so host sessions and leases close atomically.
  Live host sessions, stateless invocations, and an invocation that acquires a
  fresh session before the recheck remain untouched. Real PostgreSQL coverage
  passed the abandoned-session, fresh-session, and 129-row bounded-batch
  cases (2 tests, 1.49s), printing
  `REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration=73`; retained log:
  `/tmp/hephaestus-gateway-recovery-real-strict.log`. Supervisor scheduling,
  cancellation wiring, and service runtime execution remain pending.

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
