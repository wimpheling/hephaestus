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

## Reviewed supervisor integration plan (implementation pending)

The application will share one fresh daemon owner and one warm registry between
request dispatch and supervision, using the configured stable volume host ID.
The existing gateway reconciliation loop remains the scheduling owner alongside
Caddy reconciliation. It will own and join service tasks; starting a VM or
waiting for readiness must not block the Caddy reconciliation pass or API
readiness.

Each launch reserves capacity and persists its ownership claim before any
materialization. Its parent owns preparation, the prepared-instance worker,
lease renewal, and cleanup until all child work settles. Lease renewal runs
through preparation and shutdown, with conservative monotonic deadlines derived
from the start of each database operation. Lease loss unregisters the exact
instance and requests shutdown; in-flight preparation must still settle so a
late VM handle can be destroyed. Same-process cleanup failures retain the VM
handle and capacity reservation for retry. Materialization is removed only
after provider teardown is confirmed, and the durable row is marked cleaned
only after both steps succeed.

Startup registers an HTTP-ready worker before marking the durable instance
ready and promoting the desired revision. An already-active revision being
restored remains eligible while a different desired candidate is pending.
Candidate failure preserves the previous active revision. The first policy is
one warm serving instance per gateway, including while idle; no autoscaling or
idle suspension. Platform defaults will allow eight serving gateways, two
additional replacement/drain slots, at most two simultaneous preparations, and
sixteen HTTP requests per instance. All live, starting, draining, and incompletely
cleaned instances consume capacity. A gateway may have at most two concurrent
instances, and a new replacement waits until the preceding drain is cleaned.
Replacement slots are reserved so a full serving pool can still upgrade.

The initial timing policy is a 30-second ownership lease renewed every five
seconds, a 120-second startup/readiness budget, health checks every ten seconds
with shutdown after three consecutive failures, and a 30-second drain budget.
Probe and shutdown operations retain their independently bounded worker policy.
These values must be validated against platform bounds and exposed in the
supervisor policy rather than repeated as unrelated constants.

Cutover drains the prior instance using durable accepted-invocation counts for
its exact instance and fencing epoch, including requests accepted before their
handler starts. At the drain deadline, the instance becomes stopping, is
unregistered, and its remaining authority is terminalized through the existing
completion path. Revocation and ownership loss stop dispatch immediately.
Same-host daemon recovery fences expired claims and destroys/restarts their
VMs; the provider has no reconnect/adopt API. Recovery and incomplete cleanup
receive scheduling priority over new launches. Staging-tree cleanup requires
materialization quiescence. Failure backoff, durable diagnostics, recovery
integration, and real Caddy acceptance remain implementation requirements.

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

The durable service ownership ledger is now present in migration 0074 and its
gateway-edge/PostgreSQL adapters. It records one immutable gateway/revision
instance identity, deterministic `gateway-service-<instance_uuid>` provider
identity, host and daemon-incarnation ownership, fencing, lease timestamps,
and the provisioning-to-cleaned lifecycle. Claims serialize on the gateway
aggregate and accept either the enabled desired service revision or the active
service revision, so a serving instance can remain live while a replacement is
pending. Renewal and state transitions lock the instance before reading
`clock_timestamp()`; bounded same-host recovery fences expired rows, including
same-daemon recovery after a database stall. Non-cleaned failed rows retain
their unique gateway/revision reservation until cleanup is recorded.
Supervisor scheduling, VM materialization wiring, request draining, and orphan
cleanup remain pending.

The ownership adapter now provides fenced `provisioning -> starting -> ready -> draining`
transitions and readiness-gated promotion. Every mutation locks the gateway
aggregate before the exact instance, validates the owner/fence and a fresh
database clock after lock waits, and rejects draining the enabled active
revision. Promotion additionally locks and rechecks the exact release
publication row, updates only the active pointer while preserving the desired
tip, and is idempotent when the candidate is already active. The release
revocation path updates only the release row, so it cannot form an inverse lock
cycle with promotion's gateway -> instance -> release order; a concurrent
revocation is observed through the publication recheck.

Readiness promotion and revision cutover are now covered at the ownership port;
supervisor scheduling, VM materialization wiring, request draining, and orphan
cleanup remain pending.

Evidence: the exact disposable PostgreSQL run used
`HEPHAESTUS_POSTGRES_TEST_URL` and printed
`REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration=74`; the focused
ownership suite passed 5 tests in 3.50s. The retained log is
`/tmp/hephaestus-gateway-ownership-final-v3.log`. Targeted `gateway-edge` and
`gateway-postgres` Clippy runs with `-D warnings`, focused formatting, and
`git diff --check` pass.

Evidence for the lifecycle/promotion slice: the worker-role disposable
PostgreSQL run printed `REAL_POSTGRES_CONNECTED_AND_MIGRATED=1
max_migration=75` and passed 10 ownership tests in 6.76s. The lock-barrier
cases prove renewal and promotion recheck lease expiry after waiting on the
instance or release row; the suite also covers initial pending promotion,
old-active cutover, superseded and revoked candidates, legal transition
states, stale/fenced leases, drain protection, idempotent promotion, and
rollback without an outbox event. The retained log is
`/tmp/hephaestus-gateway-ownership-promotion-barrier.log`. `gateway-edge`
and the `gateway-postgres` ownership target pass strict Clippy with
`-D warnings`; focused formatting and `git diff --check` also pass.

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

The committed libkrun integration test now exercises both edge adapters over
the real guest vsock path: repeated gateway-edge HTTP exchanges at `/identity`
prove one long-lived process identity, and the declared readiness and health
paths pass through the bounded probe adapter. The raw transport assertions for
delayed responses, capacity, active-stream teardown, and VM cleanup remain in
the same scenario. The exact non-skipped real-VM harness passed one test in
7.23 seconds with runtime and cgroup cleanup verified; retained evidence is
`/tmp/heph-libkrun-integration-20260919-adapter-attempt2.log`. This proves the
adapter-to-vsock path only; prepared-worker, durable ownership/ledger,
supervisor, Caddy routing, and release UI behavior remain unchecked.

The read-only service-target ports now expose the supervisor's database view:
bounded stable-UUID pages include enabled gateways with an active or desired
`http.service.v1` revision, exact lifecycle-aware lookup supports already-owned
paused instances, and accepted invocation counts are scoped to an exact
gateway/revision pair for draining. Active and desired revisions remain
separate, so a revoked desired candidate does not replace a published serving
revision; stateless revisions are excluded. Evidence: the focused target suite
passed 2 tests against disposable PostgreSQL after migration 0074, with the
in-test marker `REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration=74` in
`/tmp/heph-service-targets-real-20260919-attempt10.log`; strict targeted
Clippy, formatting, and `git diff --check` pass. This is a read-side port and
adapter only; supervisor CAS/readiness promotion, request draining policy, and
application wiring remain pending.

The application composition root now adapts the edge service materializer to
the local immutable service runtime. It preserves the exact instance, gateway,
and revision identity, maps every verified artifact field, and exposes only
the sealed `/release` and `/run/hephaestus` mounts; cleanup rejects an identity
mismatch and maps runtime failures to a redacted edge error. Evidence: the
temporary-files app test passed with strict app Clippy and formatting checks.
The resolver still owns launch selection and has not yet been wired to this
adapter; supervisor startup, readiness promotion, teardown ordering, and Caddy
routing remain pending.

The gateway execution-target port now resolves an already accepted invocation
to either explicit stateless `http.v1` execution or an exact fenced persistent
service instance. The PostgreSQL adapter requires the immutable invocation,
route, revision, gateway, owner, and fencing binding; accepts ready and
draining instances after active-pointer cutover; requires a fresh unexpired
instance lease, a published exact release, and an active host-mediated session;
and rejects terminal sessions, revoked or expired inbound leases, revoked
releases, and unavailable service targets without falling back to stateless
execution. It returns the host-session expiry plus a remaining duration that
deducts lookup latency, and releases the database query before HTTP execution.
The direct invocation-fence mutation rejection is migration-75 schema
evidence; the lookup tests separately cover same-host/wrong-daemon and
wrong-host/same-daemon identity failures.

Evidence: the exact worker-role disposable PostgreSQL run printed
`REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration=75` and passed 4
aggregate tests covering identity, genuine session/instance/secret expiry,
draining cutover, stateless routing, terminal and release revocation, and
secret-lease revocation. The retained log is
`/tmp/hephaestus-gateway-execution-strengthened.log`; strict targeted Clippy,
owned-file formatting, and `git diff --check` pass. Exact-instance
draining/recovery queries and full supervisor wiring remain pending.

The edge now has a bounded warm-instance registry for supervisor-owned ready
service VMs. Keys include the exact immutable service identity and positive
`i64` fencing token; registration validates the deterministic VM ID, ready
state, open state watch, duplicate instance UUID, and global capacity. Each
instance has fail-fast request permits, and exchange validates the complete
HTTP policy before opening a connection while enforcing one deadline across
setup and HTTP I/O. Unregister cancels exact-key requests but retains the
global slot until outstanding exchanges quiesce; state transitions away from
`Ready`, closed watches, and caller cancellation close active streams. The
registry retains no worker handle, destroys no VM, and is not an authorization
boundary. Evidence: 10 focused registry tests and the full 46-test
`gateway-edge` library suite pass; formatting and diff checks pass. Strict
Clippy is pending a concurrent `service_execution.rs` doc-markdown fix.
Supervisor/DB authorization wiring, Caddy routing, and release UI remain
pending.

The prepared-instance worker now owns one already-provisioned VM from start
through readiness, bounded health checks, exit, shutdown, and cleanup. It uses
one startup deadline covering VM start and HTTP readiness, publishes `Ready`
only after a successful declared readiness probe, and keeps health failures
non-destructive while the instance remains ready. The parent supervisor owns
the returned worker future; no lifecycle task is detached. Dropping the last
control handle cancels startup or probing, and dropping a health request future
does not abandon the worker. Cleanup always attempts VM destruction after a
bounded stop attempt, and invokes materializer cleanup only after destruction
succeeds; failed destruction remains `CleanupIncomplete`. Evidence: seven
focused worker tests, gateway-edge Clippy with `-D warnings`, and
`cargo check --workspace --all-targets --all-features` pass. Disk recovery
removed only this worktree's rebuildable `target/debug/incremental` artifacts
after confirming no process used the target; source, logs, and commits were
preserved. Durable ownership, global supervision, request draining, Caddy
forwarding, and release UI behavior remain unchecked. The parent registry must
retain the VM handle when destroy fails or times out so same-process recovery
can retry destruction; a durable VM ID alone is insufficient while the
provider's live-handle registry still owns the instance.

The real libkrun integration scenario now provisions a second fixture VM with
the exact `gateway-service-<instance_uuid>` identity, retains its VM handle,
and runs the prepared worker as a parent-owned task. The test waits for
HTTP-derived `Ready`, exercises the worker health command, requests shutdown,
and asserts one exact resolver cleanup callback plus removal of the provider
runtime and cgroup entries. The distinct run passed one real VM test in 9.51
seconds with markers `REAL_PREPARED_SERVICE_WORKER_READY=1`,
`REAL_PREPARED_SERVICE_WORKER_HEALTH=1`, and
`REAL_PREPARED_SERVICE_WORKER_CLEANED=1`; evidence is retained at
`/tmp/heph-libkrun-integration-20260919-prepared-worker.log`. This proves the
prepared worker against the fixture launch only; release materialization,
durable ownership/fencing, Caddy routing, supervisor recovery, and request
draining remain unchecked.

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

- [x] Extend service draining counts to the exact instance and fencing epoch,
  and extend recovery eligibility to reject missing, stale-fenced, expired,
  or non-ready instances while preserving live `ready` and `draining` work.
  The fresh post-lock recheck uses the database clock for instance leases and
  terminalizes through the existing atomic completion path. Focused coverage
  includes stale-fence recovery, expired `Ready` instances, invalid state,
  caller-clock skew, live draining, exact cross-instance/gateway isolation,
  and the locked invocation remaining accepted before lock release. A shared
  database run exposed an old bounded-test assumption (`1` global leftover
  versus `5` eligible rows); the test now verifies the held fixture and drains
  all 129 owned rows in bounded passes. The fresh worker-role PostgreSQL run
  passed all 43 gateway-postgres tests, with migration markers through 75;
  strict all-target gateway-postgres Clippy and owned formatting/diff checks
  also passed. Evidence: `/tmp/hephaestus-gateway-postgres-full-worker.log`
  and `/tmp/hephaestus-gateway-postgres-clippy.log`. Supervisor scheduling,
  lifecycle integration, and live service execution remain pending.

The gateway aggregate now separates the latest declared service revision from
the serving revision with `desired_service_revision_id`. Service install and
configure operations update the desired pointer while preserving the active
pointer; stateless operations retain immediate activation and clear any
pending service candidate. Configure's `expected_revision_id` is the declared
tip, implemented as `COALESCE(desired_service_revision_id, active_revision_id)`
for stale checks and the locked compare-and-swap, so an older serving stateless
revision cannot discard a pending service candidate. Desired changes emit the
same committed gateway product-event outbox invalidation, while rejected
transactions emit none. Real disposable PostgreSQL proof passed all 8
`gateway-postgres` tests with
`REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration=73`; retained log:
`/tmp/heph-gateway-postgres-desired-real-20260919-attempt3.log`. Readiness
promotion, lifecycle supervision, and service-instance recovery remain
unchecked.

- [x] Add migration 0075 durable service-invocation binding. New accepted
  `http.service.v1` rows now lock gateway before instance, recheck the active
  route/revision and published release, lock the unique ready instance, verify
  its positive fencing token and unexpired lease with a fresh database clock,
  and persist the immutable instance/fence target before authority issuance.
  Stateless invocations retain null bindings. The migration terminalizes
  pre-binding accepted service history through the existing cleanup function,
  preserving rows while revoking their sessions and leases. Direct inserts,
  stale fences, and target mutation are rejected by database constraints and
  triggers. Worker-role PostgreSQL acceptance passed 7 tests, including held
  gateway/release lock barriers, readiness/lease denial, stateless behavior,
  and authority/target assertions; evidence:
  `/tmp/heph-gateway-postgres-worker-acceptance-20260919.log`. Ownership then
  target pagination passed 10 and 2 tests on the same disposable database;
  evidence: `/tmp/heph-gateway-postgres-worker-20260919.log`. The runtime
  authority fixture audit passed its service-session test after adding exact
  service instance bindings; evidence:
  `/tmp/heph-runtime-authority-postgres-20260919.log`. Warm request execution,
  handler cutover, lifecycle supervision, Caddy exposure, and service recovery
  remain unchecked.

- [x] Add the bounded edge execution router for accepted invocations. It
  validates the configured local service owner once, resolves the immutable
  invocation target under the original route deadline, delegates `http.v1` to
  the existing stateless handler, and sends `http.service.v1` only to the
  exact fenced warm-instance registry key. Service exchange time is capped by
  both the original route deadline and the host-mediated session budget;
  resolver, owner, fence, registry, and deadline failures fail closed without
  stateless fallback or new VM provisioning. Edge verification passed 58 unit
  tests plus the Caddy ingress test and strict all-target Clippy; evidence was
  produced in the current worktree. Application header substitution remains
  host-mediated and no platform bearer or mailbox authority is synthesized.

The parent-owned preparation boundary now resolves the exact claimed service
identity, validates the returned launch and deterministic VM ID, materializes
and provisions a stopped VM, and returns the launch together with its VM Arc
for the prepared-instance worker. Its cancellation handle requests
cancellation without dropping an in-flight resolver or provider future; late
provision success destroys the VM before exact materialization cleanup.
Provider orphan confirmation precedes materialization removal after a
provision error, and failures retain the VM Arc or materialization ownership
needed for same-process retry. Evidence: six focused preparation tests and
the full 59-test `gateway-edge` library suite passed; strict all-target
gateway-edge Clippy with `CARGO_INCREMENTAL=0`, formatting, and `git diff
--check` passed. The parent must retain and join the preparation future and
maintain its ownership heartbeat; supervisor integration and forced-shutdown
ledger recovery remain pending.

The application composition now routes accepted invocations through the
worker-role PostgreSQL execution-target resolver and the bounded warm-service
handler while preserving the existing stateless handler, authority, handoff,
private listener, and Caddy paths. One daemon owner is derived from the stable
volume host ID with a fresh incarnation UUID, and one registry reserves eight
serving gateways plus two replacement/drain slots with sixteen requests per
instance. Dispatcher wiring is complete; service startup scheduling and
supervisor retention remain pending. Evidence: `hephaestus-app` all-target
check, strict Clippy, formatting, and 58 library tests passed.

- [x] Add the caller-owned durable service lease monitor. It accepts the
  pre-claim monotonic deadline, renews only the exact instance/owner/fence,
  preserves lifecycle state updates, retries temporary storage failures
  without extending the deadline, and caps successful renewal from call start
  by the database expiry-minus-heartbeat budget. Cancellation and dropped
  control handles mark the claim unavailable and stop the caller-owned future.
  Full edge verification passed 65 unit tests plus the Caddy ingress test and
  strict all-target Clippy. Supervisor reaction to lease loss remains pending.

- [x] Add supervisor policy and explicit persistent-service capacity accounting.
  The validated defaults reserve eight serving gateways plus two replacement
  slots, allow two revisions per gateway, two simultaneous startups, and
  sixteen requests per instance. Reservations use unique exact tokens, remain
  counted through draining or cleanup failure, release startup allowance only
  after `finish_startup`, and release live capacity only through explicit
  post-cleanup completion. Capacity tests passed eight cases; isolated
  gateway-edge tests and strict Clippy passed on the committed checkpoint.

- [x] Harden the durable event-watch integration fixture against unrelated
  unpublished outbox backlog. A real PostgreSQL/NATS run with 450 older
  unpublished rows reproduced the original 5-second typed-event timeout;
  targeted publication now repeats bounded batches until the exact fixture
  rows are published, rejects missing or dead-lettered targets, and uses a
  30-second overall publication bound. The existing 5-second event receive
  assertion and disconnect/replay, duplicate-wake, Connect, and revocation
  checks remain unchanged. The focused test passed against the real services
  and app Clippy passed on the committed checkpoint.

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
