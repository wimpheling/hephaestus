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

Daemon cutover must restore an eligible active revision to Ready before starting
its desired replacement. After the replacement atomically promotes itself, an
exact durable target read authorizes draining the previous revision. Retain its
handle and capacity until physical and durable cleanup complete. Periodic exact
lookups must also inspect owned revisions that disappear from enabled-target
pages. Publication revocation or a missing exact target requires cancellation
and Stopping: ordinary `mark_draining` deliberately rejects an enabled active
revision, even when its publication has been revoked, so a drain request alone
would leave that worker running. Superseded revisions and paused gateways use
graceful draining. These daemon transitions remain pending implementation and
acceptance tests.

Startup bookkeeping owns claim futures as well as coordinator futures. It
captures both the lease deadline and total startup deadline before starting a
claim, and cancellation still waits for that claim to settle. A late claim is
retained for cleanup without provisioning a VM. An unavailable claim response
may hide a committed row, so its capacity reservation remains quarantined until
authoritative host inventory and a serialized database claim-resolution barrier
resolve that uncertainty. Only a definite claim rejection or confirmed physical
and durable cleanup releases live capacity.
The startup allowance is released once readiness is reached or the startup
operation has settled, independently of the retained live reservation.

The application must continuously poll the parent-owned job collection while
Caddy reconciliation is in progress. Cancelling an individual polling wait
must not drop a claim, coordinator, or retained cleanup responsibility. Shutdown
cancels and settles the owned jobs and preserves unresolved resources for the
existing recovery contract; it must not rely on dropping a task set to abort
provisioning. These bookkeeping and application integration requirements remain
pending until their implementation and acceptance checks are complete.

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

The next recovery integration must close the startup admission gate until a
complete bounded host-inventory pass accounts for prior daemon instances.
Unexpired rows belonging to an earlier daemon remain unresolved until the
database permits an expired-claim takeover; a local clock comparison alone is
not permission to destroy their VM. Recovery must use the returned fenced
lease, settle provider orphan cleanup before materializer cleanup, and confirm
the durable cleaned state before considering that resource released. Inventory
pagination and recovery concurrency remain bounded even if historical rows
exceed the current launch capacity. Such a backlog blocks new launches while
cleanup makes progress; it must not be discarded to fit the capacity limit.

For rows already tracked by this daemon, recovery must reuse the existing job,
capacity token, and retained VM handle. In particular, an expired same-process
claim cannot be routed through provider orphan cleanup while its VM remains
registered. A claim-resolution barrier may release an uncertain reservation
only after it proves the original claim did not commit; an ordinary inventory
page that omits the row is insufficient. These are pending integration
requirements, not acceptance evidence for the current startup primitives.

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

The parent-owned service instance coordinator now composes lease monitoring,
preparation, the prepared worker, registry registration, readiness-gated
promotion or active-revision restore, and explicit cleanup for one claimed
instance. Its startup deadline covers preparation through HTTP readiness;
lease loss and cancellation unregister the exact instance and still await
in-flight preparation or worker teardown. Durable stopping and cleaning are
reported only after physical cleanup is confirmed, while failed cleanup
retains the exact VM handle, materialization ownership, latest lease, and
primary failure reason. Focused tests cover blocked cleanup with continued
lease renewal, late VM destruction after lease loss, failed readiness and
promotion, worker exit during promotion, restore with a newer desired
revision, blocked restore-query lease loss, and retained cleanup ownership.
Evidence: 10 coordinator tests and the full 83-test `gateway-edge` library
suite pass; strict all-target/all-feature gateway-edge Clippy, package checks,
and `git diff --check` pass. Global supervision, health/drain policy,
durable failure-store reporting, Caddy routing, and release UI integration
remain pending.
The coordinator ownership integration run used disposable PostgreSQL after
migration 0076, printed `REAL_POSTGRES_CONNECTED_AND_MIGRATED=1
max_migration=76`, and passed 16 real ownership/coordinator tests; the
retained log is `/tmp/hephaestus-gateway-ownership-coordinator-full.log`.

The parent-owned service worker now exposes a read-only redacted failure
snapshot on its control handle. It records the primary startup, readiness, or
unexpected-exit category before teardown, retains valid bounded VM exit code
or signal metadata through cleanup failure, and drops malformed provider
metadata to category-only `unexpected_exit`; requested shutdown leaves no
failure snapshot. Focused tests cover startup failure and timeout,
readiness timeout, requested shutdown, exit code and signal retention through
destroy failure, and malformed exit redaction. In an isolated checkout at
`3c7f538` with only this slice applied, `cargo test -p gateway-edge
--all-features` passed 86 library tests plus one Caddy integration test,
strict all-target/all-feature gateway-edge Clippy passed, and formatting and
`git diff --check` passed. Durable failure-store reporting and supervisor
publication remain pending.

The prepared-worker libkrun integration now requests the fixture `/crash`
endpoint, verifies the owned worker reports redacted `UnexpectedExit` with the
actual exit code `42`, and confirms VM, cgroup, and materializer cleanup. It
then provisions a fresh instance of the same gateway and revision, verifies a
different guest startup identity reaches readiness and serves `/identity`, and
shuts that replacement down cleanly with bounded worker joins. The documented
command `HEPHAESTUS_LIBKRUN_INTEGRATION=1 ./scripts/run-libkrun-integration.sh`
passed one real hardware test in 11.69 seconds; markers and cleanup evidence
are retained at `/tmp/heph-libkrun-integration-20260919-replacement-worker-final.log`.
This proves prepared-worker crash diagnostics and same-service replacement;
daemon/global restart recovery and Caddy acceptance remain outside this slice.

The gateway edge now has a parent-owned retryable physical-cleanup helper for
one exact service identity. It retains the same VM handle after a failed or
timed-out destroy, uses deterministic orphan cleanup when no handle survives,
and retries only materializer cleanup after provider teardown is confirmed.
Four focused tests cover retained-handle retry, orphan-ID and materializer
identity scope, materializer-only retry, exact input rejection, and bounded
pending destroy. In isolated checkpoint `3c02378` with only this helper
applied, the focused tests and strict gateway-edge all-target/all-feature
Clippy passed; formatting and `git diff --check` passed. This is a physical
cleanup primitive only; durable cleanup state, capacity release, and
supervisor scheduling remain the caller's responsibility.

The fenced cleanup driver now composes that physical helper with the durable
lease monitor, redacted failure store, exact-instance lookup, and fenced
`mark_cleaned` transition. It retains all caller-owned cleanup, lease, and
failure state on unavailable or stale returns; physical cleanup still settles
after lease loss, while durable writes stop. Eight focused driver tests cover
heartbeat renewal during blocked cleanup, report-before-clean transition,
unavailable reporting, ambiguous completion confirmation, stale fences,
expired cleaned-row confirmation, and lease-loss settlement. An exact
`49c8ecc` checkout with only this slice overlaid passed 101 gateway-edge
library tests, strict all-target/all-feature Clippy, formatting, and diff
checks. This remains a cleanup
driver only; scheduler, claim recovery, capacity release, and Caddy routing
remain pending.

The claim-resolution port now resolves an ambiguous non-cleaned service claim
only after taking the same gateway-row lock used by new claims and issuing a
fresh `READ COMMITTED` lookup. Missing gateways and cleaned rows resolve to an
absence only after that barrier; expired claims and newer owner/fence epochs
remain visible for recovery. Four real worker-role PostgreSQL tests passed
against migration 0076, including commit and rollback lock barriers, expiry
before recovery, cleaned-row exclusion, and nil-identity rejection; the
in-test marker is retained in `/tmp/heph-claim-resolution-real-v8.log`.
Strict `gateway-edge` and `gateway-postgres` Clippy with `-D warnings`, the
101-test edge library baseline, formatting, and `git diff --check` pass in an
isolated checkout at baseline `265d718`. Supervisor integration, claim
recovery scheduling, Caddy routing, and release UI remain pending.

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

- [x] Wire bounded abandoned-invocation recovery into the existing daemon
  `gateway_reconciliation_loop`, using a separate worker-role authority pool so
  the existing Caddy/dispatcher authority remains unchanged. The loop keeps at
  most one recovery batch in flight, continues Caddy reconciliation while the
  database batch waits, and aborts and joins the database task during daemon
  shutdown. Two real worker-role PostgreSQL app tests passed against migration
  0076: a stopping instance terminalized its invocation and revoked its host
  session and secret lease while a live Ready instance and all three of its
  records remained active; a held PostgreSQL row demonstrated worker lock
  waiting, Caddy progress, cancellation, and clean joining. Evidence:
  `/tmp/hephaestus-reaper-overlay-real-20260919-rerun.log`,
  `/tmp/hephaestus-reaper-overlay-app-lib-20260919.log`, and
  `/tmp/hephaestus-reaper-overlay-clippy-20260919.log`. Workspace formatting
  still has unrelated baseline diffs in the service-instance and supervisor
  files; app formatting and strict app Clippy passed. Full service supervisor
  startup, claim recovery, draining, and Caddy acceptance remain pending.

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

- [x] Add serving-health supervision to the parent-owned service coordinator.
  The coordinator now consumes the validated supervisor policy, schedules one
  bounded health probe at a time while continuing to observe worker exit,
  lease loss, and cancellation, resets consecutive failures after a successful
  probe, and tears down with an explicit health failure at the configured
  threshold. Thirteen focused coordinator tests cover reset proof, blocked
  probe cancellation and lease loss, and cleanup; the full gateway-edge
  library passed 89 tests, strict all-target Clippy, formatting, and diff
  checks. The two real PostgreSQL coordinator tests passed with the in-test
  `REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration=76` marker in
  `/tmp/hephaestus-gateway-health-coordinator-real.log`. Durable failure
  recording is wired by the coordinator checkpoint below; draining and global
  supervisor scheduling remain pending.

- [x] Connect coordinator failures to the durable failure store before
  `mark_cleaned`. Worker snapshots preserve readiness, health, and bounded
  process-exit metadata; preparation, startup-deadline, and cleanup failures
  receive redacted categories, while cancellation and lease loss alone do not
  increment retry backoff. Reporting is lease-deadline-bounded and keeps the
  lease monitor running; unavailable reporting or stopping transitions retain
  the pending report and prevent false durable cleanup. Thirteen focused
  coordinator tests passed, including blocked reporting renewal, physical
  cleanup with unavailable storage, and stopping-failure retention. In an
  isolated checkout at `be3a281`, the full gateway-edge library passed 93
  tests, strict edge and PostgreSQL all-target/all-feature Clippy passed, and
  formatting/diff checks passed. Three real PostgreSQL coordinator tests passed
  with the in-test `REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration=76`
  marker; the log is `/tmp/heph-failure-verify-real.log`. The failure test
  confirmed retry backoff, report-before-cleaned ordering, and preservation of
  the active and desired revision pointers.

- [x] Add coordinator graceful drain for accepted service invocations. A
  parent drain request is coalesced, fenced `mark_draining` conflicts preserve
  `Ready` without retry spin, and the exact instance/fencing count remains
  authoritative while the registry continues dispatching already accepted
  calls. The total drain budget includes the durable transition, bounded
  unavailable-count retries, and teardown; normal drained or deadline
  retirement does not record failure backoff. Focused edge coverage now has
  107 tests, strict edge/PostgreSQL all-target Clippy and formatting pass, and
  the three real PostgreSQL coordinator tests passed with
  `REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration=76`; log:
  `/tmp/heph-drain-review-pg.log`. Global reaping, supervisor scheduling,
  Caddy cutover/exposure, and release UI integration remain pending.

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
  post-cleanup completion. Capacity tests passed eight cases, and
  `CARGO_INCREMENTAL=0 cargo clippy -p gateway-edge --all-targets
  --all-features -- -D warnings` passed after the committed checkpoint.

- [x] Harden the durable event-watch integration fixture against unrelated
  unpublished outbox backlog. A real PostgreSQL/NATS run with 450 older
  unpublished rows reproduced the original 5-second typed-event timeout;
  targeted publication now repeats bounded batches until the exact fixture
  rows are published, rejects missing or dead-lettered targets, and uses a
  30-second overall publication bound. The existing 5-second event receive
  assertion and disconnect/replay, duplicate-wake, Connect, and revocation
  checks remain unchanged. The focused test passed against the real services
  and app Clippy passed on the committed checkpoint.

- [x] Add durable service failure recording and retry backoff. Failure reports
  use bounded redacted categories and optional provider exit values; the first
  report for an exact live owner is write-once, updates a revision-scoped
  exponential backoff, and never stores raw logs. Claim and readiness paths
  lock the gateway and retry state in order, recheck the database clock after
  lock waits, and fail closed on stale owners. Real worker-role PostgreSQL
  evidence passed four focused failure tests and all 14 ownership tests
  against migration 0076; strict targeted gateway-postgres Clippy passed.
  Logs: `/tmp/hephaestus-gateway-failure-worker.log` and
  `/tmp/hephaestus-gateway-ownership-worker.log`.

- [x] Add exact durable service-instance lookup for recovery. The worker-role
  target adapter now looks up the immutable instance, gateway, and revision
  identity without filtering by fencing token, so recovery can observe a
  newer owner epoch and already-cleaned rows. The row conversion reuses the
  ownership adapter's validated state mapping. Real PostgreSQL coverage
  exercised `starting`, `stopping`, expired claim recovery with a fence and
  owner change, `cleaned`, wrong identities, and all nil identity fields;
  the full target suite passed three tests against migration 0076. Evidence:
  `/tmp/hephaestus-gateway-service-target-lookup-20260919.log`. Commands:
  `HEPHAESTUS_POSTGRES_TEST_URL=postgres://postgres:postgres@127.0.0.1:33179/hephaestus?sslmode=disable
  CARGO_INCREMENTAL=0 cargo test -p gateway-postgres --test service_targets
  -- --nocapture --test-threads=1`
  and targeted strict checks for `gateway-edge` plus the PostgreSQL library
  and `service_targets` test. Supervisor recovery scheduling and lifecycle
  integration remain pending.

- [x] Add bounded stable-host inventory for persistent service instances. The
  worker-role target adapter now pages non-cleaned rows by stable host ID and
  immutable instance UUID, deliberately retaining expired claims and older
  daemon owner UUIDs while excluding other hosts and cleaned rows. A
  randomized real PostgreSQL fixture covered 130 same-host instances,
  multiple owner UUIDs, live and expired leases, foreign-host exclusion,
  cleaned-row exclusion, strict cursor ordering, and invalid page inputs;
  all four target tests passed against migration 0076. Evidence:
  `/tmp/hephaestus-gateway-service-target-inventory-20260919.log`. Full
  gateway-postgres all-target strict Clippy and targeted edge/postgres checks
  passed; full edge all-target Clippy remains blocked by unrelated coordinator
  test lints. This inventory supports capacity accounting and previous-daemon
  recovery; it does not claim global recovery or lifecycle scheduling.

- [x] Add a repository-native ordinary HTTP release fixture at
  `examples/cooking/cooking-service`. The dependency-free service binds only
  `127.0.0.1:8080`, has bounded workers, queue, headers, and absolute
  header-read/write deadlines, and exposes readiness, health, stable identity,
  and simple public responses. Its v2 agent and `http.service.v1` gateway
  manifests pass the real `agent-config` parser test. Native tests (4), strict
  example Clippy, formatting, rustdoc, locked offline release build, and a
  repeated-identity curl smoke passed. The production `build.sh` was not run
  on the host because it intentionally requires `/opt/cargo` and `/opt/rust`
  from the pinned builder image; the equivalent locked offline Cargo build
  passed. Caddy forwarding, managed lifecycle, and publish/install CLI support
  remain pending and are not claimed by this fixture.

- [x] Bound final daemon outbox flushing to the existing shutdown deadline.
  The old 100-pass cap could leave 974 product rows pending when the serial
  workspace suite accumulated more than 2,000 rows; the exact baseline
  reproduction failed at `golden.rs:2997` in
  `/tmp/heph-golden-4b4b091-full.log`. The deadline-driven loop preserves the
  existing quiescence error, and focused unit tests cover 101 passes plus an
  already-expired deadline. An isolated committed-base full workspace run
  with the equivalent deadline loop passed in
  `/tmp/heph-golden-4b4b091-dynamic.log` (this is not a claim about the current
  concurrent worktree). The final focused real PostgreSQL/NATS golden run
  passed in `/tmp/heph-shutdown-focused-final.log`; its post-shutdown census
  reports `pending_product=0`, `published_product=167`, and `dead_product=0`
  in `/tmp/heph-shutdown-focused-final-db-census.log`. The focused app tests,
  formatting, and clean-base strict app Clippy all passed. Full current-head
  workspace quality remains pending concurrent gateway work.

- [x] Add bounded lifecycle diagnostics to the parent-owned service worker.
  The worker subscribes to provider events before VM start and exposes a typed
  watch snapshot containing lifecycle milestones, stdout/stderr byte counts,
  lagged-event counts, and channel closure. Raw log bytes, request content,
  credentials, and arbitrary metric labels are discarded; event lag or channel
  closure does not fail the service or starve cancellation and cleanup. This
  does not deliver application log chunks or complete the security/logging
  checklist: project-scoped application logs remain an explicit opt-in contract
  with app-owned redaction, bounded retention, and a later authorized stream.

- [x] Add parent-owned concurrent service startup bookkeeping. The supervisor
  captures deadlines before claim, reserves capacity before any durable call,
  owns and settles claim/coordinator futures through cancellation, quarantines
  unavailable claims, releases startup capacity at readiness, and retains
  unresolved leases, VM handles, and cleanup state until explicit shutdown.
  Its readiness fixture exercises the coordinator through a private HTTP probe
  and verifies destroy failure retains the exact VM handle and live capacity.
  Isolated committed-base evidence passed 11 supervisor tests, 112 gateway-edge
  library tests, strict all-target/all-feature Clippy, and formatting. The
  supervisor module export follows the capacity exports, preserving the
  committed-head rustfmt ordering. This is the bookkeeping building block;
  application polling/reconciliation, recovery scheduling, and Caddy routing
  remain pending.

- [x] Add parent-owned retries for terminal coordinator cleanup. A retry
  renews the exact owner and fence under a conservative bounded deadline,
  monitors the `Stopping` transition, and reuses the original capacity token
  and parent-owned future collection. `GatewayServiceCleanup` progress,
  retained VM handles, pending redacted failures, cleanup flags, and the
  original coordinator reason survive every failed attempt and shutdown.
  Already-confirmed physical cleanup uses an exact read-only `Cleaned` lookup;
  a coordinator `vm=None` result alone never proves teardown, so the first
  retry conservatively performs orphan confirmation. Tests cover retry with
  the same VM handle and no orphan cleanup, materializer-only retry without
  redestroy, unavailable failure reporting, stale renewal without physical or
  durable writes, exact cleaned confirmation, and another startup progressing
  while cleanup is blocked. This remains a bounded retry primitive: target
  scanning, new claims, expired-host recovery, ambiguous-claim resolution,
  and application scheduling remain pending.

  Exact verification used committed base `69c778d7e70348246b8f99c435c78654512d1b79`
  in `/tmp/heph-retry-check-20260919b`, with only these shared-worktree
  overlays copied into that checkout: `crates/gateway-edge/src/service_cleanup.rs`,
  `crates/gateway-edge/src/service_cleanup_driver.rs`,
  `crates/gateway-edge/src/service_instance.rs`, and
  `crates/gateway-edge/src/service_supervisor.rs`. Workspace formatting passed
  in `/tmp/heph-retry-check-20260919-fmt.log`; 128 gateway-edge library tests
  passed in `/tmp/heph-retry-check-20260919-tests.log`; strict
  `cargo clippy -p gateway-edge --all-targets --all-features -- -D warnings`
  passed in `/tmp/heph-retry-check-20260919-clippy.log`.

- [x] Reconcile terminal claims that did not enter the coordinator. The
  supervisor now polls startup, cleanup, and serialized claim-resolution
  futures fairly; validates the complete instance, VM, host, daemon, and fence
  identity; preserves the original known lease when a resolution reports a
  foreign, newer, malformed, expired, or unavailable claim; and sends
  `Settled` only after authoritative capacity release. An owned late claim is
  routed through the existing cleanup state, while claim-resolution polling is
  cancellation-safe and shutdown retains unresolved requests and reasons.
  Isolated committed-base evidence used `2d5b8ae` with only
  `crates/gateway-edge/src/service_supervisor.rs` overlaid. The focused
  supervisor suite passed 19 tests, the full gateway-edge library passed 134
  tests, strict gateway-edge all-target/all-feature Clippy passed, and Cargo
  formatting passed. Logs are retained at
  `/tmp/heph-reconcile-check-20260919-full-library.log`,
  `/tmp/heph-reconcile-check-20260919-clippy.log`, and
  `/tmp/heph-reconcile-check-20260919-fmt.log`. Application polling,
  inventory-based recovery, and global scheduling remain pending.

- [x] Add exact same-host takeover for one expired service instance. The new
  recovery port and PostgreSQL adapter lock gateway then the exact
  `(instance_id, gateway_id, revision_id)` row, re-read `clock_timestamp()`
  after both locks, reject live/cleaned/foreign/mismatched rows, and return a
  validated `Stopping` lease with a new fencing epoch and daemon owner. The
  real worker-role suite covers live rejection, foreign host and wrong scope,
  an expired cleaned row, successful fencing, and competing takeovers with one
  winner. Evidence used committed base `2d5b8ae` plus only the takeover
  overlay; four tests passed against migration 0077 with the connected marker
  in `/tmp/heph-exact-takeover-isolated-real-final-20260919.log`. Broad
  gateway-edge/gateway-postgres all-target/all-feature Clippy passed in
  `/tmp/heph-exact-takeover-isolated-broad-clippy.log`, and isolated workspace
  formatting passed. The boot gate that pages inventory and drives this port
  remains pending.

- [x] Add the default-off, release-scoped application log policy to the typed
  service declaration and immutable revision storage. The `disabled` default
  is omitted from normalized JSON/TOML serialization, preserving the frozen
  pre-change service hash; explicit `application` changes the hash. Migration
  0077 persists the closed policy, rejects unknown values, keeps stateless
  revisions disabled, and carries the value through install, configure-copy,
  management reads, worker target reads, and worker launch resolution. Real
  PostgreSQL evidence passed 8 gateway tests and 4 worker target tests with
  migration marker 77; evidence is retained in
  `/tmp/heph-storage-real-20260919.log` and
  `/tmp/heph-storage-targets-final-20260919.log` using isolated overlay base
  `5f0f460ef402105f6fec9b551e07ab51782c2a3d`.
  Application capture, bounded retention, authorized scoped reading, and RPC
  exposure remain pending; lifecycle diagnostics remain content-free.

- [x] Add the first opt-in application log collector boundary to the service
  worker. `Application` declarations create a parent-visible bounded queue;
  `Disabled` declarations retain no raw bytes. Each event is capped at 64 KiB,
  the queue at 64 chunks and 4 MiB, and oversized, full-queue, contention,
  sequence-exhaustion, and provider-event-lag loss are reported explicitly.
  Records expose stream, host-observed time, and sequence metadata; custom
  debug output reports only metadata and byte lengths, so raw chunks do not
  enter tracing or lifecycle failure metadata. The worker continues its same
  lifecycle/readiness/cleanup future while capture uses synchronous bounded
  `try_record` and a bounded drain API for the future durable writer.
  Application-enabled readiness/flood tests, disabled-mode coverage, queue
  overflow, provider lag, contention, and sequence exhaustion passed. Clean
  committed-base `ff57c9f` with only the three collector files overlaid passed
  134 gateway-edge library tests, strict all-target/all-feature Clippy,
  workspace formatting, and workspace rustdoc; logs are retained at
  `/tmp/heph-log-ff57c9f-verified/`. Durable PostgreSQL writing, retention
  policy, authorized scoped reading, and RPC exposure remain pending.

## Non-goals

This task does not replace MVP 03's bounded stateless invocation mode. It does
not grant guest Caddy admin access, public host ports, arbitrary listener
binding, or ambient project/network authority.

## Related work

- [MVP 03: Gateway HTTP routing and invocation](../done/mvp-03-event-ingress-and-caddy-routing.md)
- [MVP 03.1: Gateway principals and authority](../done/mvp-03.1-gateway-principals-and-authority.md)

## Verification gates

Next daemon integration acceptance boundary: after boot recovery proves a fresh
empty host inventory, the existing reconciliation loop must select real durable
targets and retain startup handles; constructing a supervisor alone does not
launch services. Target reads must remain bounded and must not stop polling
leases, cleanup, or Caddy. Restore an eligible active service before attempting
its desired replacement. Retain job identity and capacity through uncertain
claims and incomplete cleanup, and reconcile those states before admitting more
work. Forward drain requests through retained handles after a successful
replacement; do not treat cancellation as graceful draining. Check owned targets
explicitly for pause, removal, or publication revocation, since they disappear
from the enabled-target listing. Verification must exercise this actual daemon
selection path rather than pre-starting a supervisor job through a test seam.

Current integration investigation (2026-09-19): running the PostgreSQL fixture
suite before the app recovery test reproduces a route validation mismatch.
Persisted routes such as `/http.service.v1` pass the database contract but fail
edge validation, so `desired_configuration()` fails before Caddy reconciliation
starts. Evidence: `/tmp/heph-sequential-gateway-postgres-diagnostic-20260919.log`.
The validator fix now accepts ordinary dotted segments while preserving
traversal and normalization rejection. Isolated focused tests, strict
gateway-edge all-target/all-feature Clippy, and formatting passed. Against a
fresh migration-77 database, all seven gateway-postgres test binaries passed
(60 tests total), followed by the app recovery regression with an in-test
`REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration=77` marker. Logs:
`/tmp/heph-route-validator-gateway-postgres-20260919-v2.log`,
`/tmp/heph-route-validator-db-marker-20260919.log`, and
`/tmp/heph-route-validator-app-recovery-20260919-v2.log`. Increasing the
recovery test timeout does not address this failure. Boot recovery and the
migration 0078 durable log append slice are still under implementation/review;
the latter's initially skipped PostgreSQL test is not acceptance evidence.

The app supervisor composition checkpoint is verified on isolated `da1f16d`
with only the app overlay and migration expectation 77. `GatewayEdgeRuntime`
now shares the service owner, registry, materializer, and supervisor context;
the reconciliation loop parent-polls one Caddy reconciliation future, coalesces
recovery requests, and polls the service supervisor while jobs are pending.
The blocked-Caddy real PostgreSQL fixture reached service `Ready`, advanced its
heartbeat, then cancelled and cleaned the instance. Full app library tests
passed (63 tests) against migration 77. Rust 1.88.0 workspace Clippy,
formatting, and rustdoc passed in the same overlay. Durable target selection,
startup scheduling, boot recovery, and global supervisor policy integration
remain pending.

Evidence: `/tmp/heph-app-supervisor-full-real-pinned-final-20260919.log`,
`/tmp/heph-app-supervisor-db-marker-final-20260919.log`,
`/tmp/heph-app-supervisor-workspace-clippy-pinned-20260919.log`,
`/tmp/heph-app-supervisor-fmt-pinned-20260919.log`, and
`/tmp/heph-app-supervisor-doc-pinned-final-20260919.log`.

The supervisor drain-forwarding checkpoint is verified on clean `2f63d50` with
only `service_supervisor.rs` overlaid. `GatewayServiceStartupHandle` now
coalesces drain requests made before coordinator creation or while the job is
ready, while cancellation and parent-owned futures remain independent. The
accepted-count regression holds one invocation during drain, proves the job
stays ready and pending, then releases the count and verifies cleanup. The
isolated edge library passed 143 tests; strict Rust 1.88.0 edge Clippy,
formatting, and workspace rustdoc passed. Actual daemon target selection and
startup scheduling remain pending.

Evidence: `/tmp/heph-drain-forward-isolated-edge-lib-20260919.log`,
`/tmp/heph-drain-forward-isolated-clippy-20260919.log`,
`/tmp/heph-drain-forward-isolated-fmt-20260919.log`, and
`/tmp/heph-drain-forward-isolated-doc-20260919.log`.

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

Append checkpoint (2026-09-19): migration 0078 and the worker-only PostgreSQL
append adapter now persist opt-in application service logs with exact
instance/fencing identity, quota and epoch metadata, ordered sequence
watermarks, duplicate replay handling, and explicit producer/provider/storage
loss counters. The adapter uses the quota -> gateway -> instance -> epoch lock
order, validates the fresh lease and capture mode, and keeps payloads behind
project RLS. Metadata-cap counters are rejected-submission totals: an
ambiguous commit followed by retry can count the same bytes again because no
epoch watermark exists for a rejected batch; `Capacity` is terminal for an
acknowledged batch while `Unavailable` remains ambiguous. Real PostgreSQL
coverage passed two append tests after migration 0078 with
`REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration=78`; pinned Rust 1.88.0
edge/PostgreSQL all-target Clippy, the focused batch contract test, formatting,
and workspace rustdoc also passed. Durable writer/flush integration,
maintenance/eviction, authorized readers/RPC, and app/supervisor integration
remain pending.

Boot recovery checkpoint (2026-09-19): the parent-owned gate now proves a
fresh first-page host inventory is empty before returning `Complete`, recovers
at most two exact service claims concurrently, renews both leases while
physical teardown is blocked, and retains raw ownership across dropped polling,
shutdown, malformed batch responses, unavailable acknowledgements, and partial
renewal failures. The 12-instance regression records every deterministic VM ID
as physically cleaned and every durable row as cleaned before the next fresh
empty proof; exact takeover rejects changed identity, VM ID, owner, or fence.
The isolated overlay is committed `2f63d50` plus only the boot module/export
changes. Rust 1.88.0 passed 152 gateway-edge unit tests and one ingress test,
strict all-target/all-feature Clippy, formatting, and workspace
rustdoc. Logs:
`/tmp/gateway-edge-boot-isolated-test.log`,
`/tmp/gateway-edge-boot-isolated-clippy.log`,
`/tmp/gateway-edge-boot-isolated-fmt.log`, and
`/tmp/gateway-edge-boot-doc.log`. Daemon boot wiring, target scheduling, and
later host inventory integration remain pending; this checkpoint does not mark
the persistent-service feature complete.

Automatic startup checkpoint (2026-09-19): the app reconciliation loop now
constructs the reviewed boot gate and polls it alongside Caddy, recovery, and
the parent-owned service supervisor. After a fresh host-inventory proof, it
performs bounded target reads and starts an eligible active service for restore
or an eligible desired service when no active service exists; it retains startup
handles until capacity is released and does not replace an already-serving
revision. The isolated `725f734` overlay passed the full app library suite
(65 tests) and the focused gateway recovery suite (5 tests) against disposable
worker-role PostgreSQL databases with in-test
`REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration=78` markers. Rust 1.88.0
strict app Clippy, formatting, and diff checks passed. Evidence:
`/tmp/heph-app-startup-full-real-pg-final4.log`,
`/tmp/heph-app-startup-focused-real-pg-final3.log`, and
`/tmp/heph-app-startup-clippy-final5.log`. Desired cutover while an active
revision is serving, revocation/drain scheduling, restart recovery beyond the
boot gate, and real Caddy/guest acceptance remain pending.

Writer checkpoint (2026-09-19): `ServiceLogWriter` is now a parent-owned,
immutable instance/owner/fence pump. It drains at most one 64-record/4 MiB
batch, retains the exact batch across unavailable or cancelled appends, uses
bounded retry cadence, flushes cumulative loss counters without duplication,
and reports terminal stale/contract failures and known unacknowledged data.
Capacity accounts one batch and permits later batches; loss-only retries remain
pending until acknowledged. `final_flush` is deadline-bound and must run after
the worker event collector has settled while lease supervision remains active;
idle polls are caller-paced. Exact overlay base `725f734` plus only the writer
module/export passed 162 gateway-edge tests, pinned Rust 1.88.0 all-target
Clippy, formatting, and workspace rustdoc. Logs:
`/tmp/heph-log-writer-edge-tests-final2-20260919.log`,
`/tmp/heph-log-writer-edge-clippy-final2-20260919.log`,
`/tmp/heph-log-writer-fmt-final2-20260919.log`, and
`/tmp/heph-log-writer-workspace-doc-final-20260919.log`. Coordinator/app
writer wiring, durable retention/maintenance, and authorized readers remain
pending.

Lifecycle writer checkpoint (2026-09-19): the coordinator and supervisor now accept an
optional parent-owned `GatewayServiceLogWriterConfig` and drive the immutable writer
alongside the worker without detached tasks. Worker teardown completes before the
deadline-bound final flush, and durable `cleaned` follows flush acknowledgement or the
bounded deadline; logging failures remain non-fatal and emit only redacted identity,
count, and loss metadata. Coordinator tests cover application capture while Ready with
health and lease activity, disabled capture with zero store calls, blocked final flush
ordering, and stale-fence termination. Exact overlay base `b26f880` plus the four
gateway-edge lifecycle files passed 168 gateway-edge tests, pinned Rust 1.88.0 strict
all-target Clippy, formatting, and workspace rustdoc. Logs:
`/tmp/heph-service-log-lifecycle-edge-tests-final3-isolated-20260919.log`,
`/tmp/heph-service-log-lifecycle-edge-clippy-final3-isolated-20260919.log`,
`/tmp/heph-service-log-lifecycle-fmt-final3-isolated-20260919.log`, and
`/tmp/heph-service-log-lifecycle-doc-final3-isolated-20260919.log`. The production app
has not yet attached the PostgreSQL log store; retention/maintenance and authorized
readers/RPC remain pending.

Fixture acceptance checkpoint (2026-09-19): the committed startup base
`f68257d` was archived into
`/tmp/heph-persistent-fixture-verify-20260919`; only
`crates/hephaestus-app/tests/golden.rs` and
`crates/vm-libkrun/src/bin/heph-integration-check.rs` were overlaid. The
joined command
`HEPHAESTUS_APP_GATEWAY_SERVICE_E2E=1 scripts/run-gateway-libkrun-e2e.sh`
ran as the real libkrun/Caddy acceptance proof in
`/tmp/heph-persistent-service-e2e-20260919-attempt5-isolated.log`.
The golden suite passed 35 tests with one ignored; the public proof recorded
`persistent-service-public identity_equal=true pid=339
startup_id=339-1789805352599931647 request_count=2->3`, followed by
`persistent-service-e2e=passed`. The gateway-postgres suite passed all 8 tests,
including `REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration=78`,
`REAL_POSTGRES_SERVICE_RESOLVER_FIXTURE=seeded`, and
`REAL_POSTGRES_SERVICE_RESOLVER_QUERY=passed`. The wrapper confirmed
`daemon golden E2E passed; runtime and cgroup cleanup verified`.
Pinned Rust 1.88 verification of the final fixture overlay passed formatting,
golden compilation, strict targeted Clippy, and rustdoc; logs are
`/tmp/heph-persistent-fixture-fmt-f68257d.log`,
`/tmp/heph-persistent-fixture-compile-f68257d.log`,
`/tmp/heph-persistent-fixture-clippy-f68257d.log`, and
`/tmp/heph-persistent-fixture-doc-f68257d.log`. The assertion proves only the
initial warm service path: restart recovery, desired cutover, revocation/drain,
adversarial behavior, and release UI remain pending.

Graceful restart acceptance checkpoint (2026-09-19): on committed base
`c9904ee`, the isolated overlay
`/tmp/heph-persistent-restart-verify-20260919-v2` ran the joined command
`HEPHAESTUS_APP_GATEWAY_SERVICE_E2E=1 scripts/run-gateway-libkrun-e2e.sh`.
The real libkrun/Caddy golden proof recorded the first service instance
`1b94d3b4-b2dd-43d9-a0d3-08e3af492d28`, startup ID
`341-1789806313227606957`, and request count `2->3`; after graceful daemon
shutdown, durable `cleaned` state, and runtime/cgroup/materializer absence, the
same database/configuration restored the same immutable revision as new service
instance `b7c0438d-ea53-4f4e-8f67-c246a77c25c0`, startup ID
`342-1789806316737385297`, and request count `2->3`. Both public pairs used one
stable process identity within each lifecycle, and Caddy restored the service
route after restart. The run passed 35 golden tests with one ignored and the
8-test gateway-postgres resolver suite; it emitted
`persistent-service-e2e=passed` and
`daemon golden E2E passed; runtime and cgroup cleanup verified`. Final pinned
Rust 1.88 checks on the isolated overlay passed formatting, golden compilation,
strict targeted Clippy, and rustdoc. Logs:
`/tmp/heph-persistent-service-restart-e2e-20260919.log`,
`/tmp/heph-persistent-restart-compile-final-isolated-20260919.log`,
`/tmp/heph-persistent-restart-clippy-final-isolated-20260919.log`, and
`/tmp/heph-persistent-restart-doc-final-isolated-20260919.log`. This proves
graceful daemon restart and active-service restoration only; unclean crash,
expired-boot recovery, cutover, revocation/drain, and release UI remain pending.

CI fixture-isolation checkpoint (2026-09-19): the exact `f68257d` application
source was verified unchanged except for the recovery test file in the
isolated overlay `/tmp/heph-ci-isolation`. The patch only gives the three
automatic-start tests unique disposable PostgreSQL databases, applies the
current migrations, connects their service ports with the
`hephaestus_worker` role, and drops each database after the test. It contains
no cutover test, selector change, or production application change. After a
fresh gateway-postgres database was populated by the full seven-binary suite,
62 gateway-postgres tests passed (9+8+7+4+25+3+6); the original f682 recovery
group then passed 5 tests, and the full app library passed 65 tests. Each
isolated startup test logged a distinct database name, worker role, migration
78, and a matching drop marker. Pinned Rust 1.88 formatting, strict app
all-target/all-feature Clippy, and app rustdoc passed. Evidence:
`/tmp/heph-ci-isolation-gateway-postgres-fresh.log`,
`/tmp/heph-ci-isolation-app-five.log`,
`/tmp/heph-ci-isolation-app-full.log`,
`/tmp/heph-ci-isolation-app-clippy-v2.log`,
`/tmp/heph-ci-isolation-fmt.log`, and
`/tmp/heph-ci-isolation-app-doc.log`. The original CI failure was shared
database fixture contamination: enabled targets left by the PostgreSQL suite
could consume the bounded global startup scan before the recovery fixture was
reached. The tested fix is test isolation only; the cutover implementation
remains a separate uncommitted work in progress.
Committed verification at `b695842` is also complete: [CI run
35432539567](https://github.com/wimpheling/hephaestus/actions/runs/35432539567)
passes Rust/authorization, cooking applications, and live browser review.

Payload pagination checkpoint (2026-09-19): the authorized PostgreSQL reader now
uses one repeatable-read transaction for metadata, candidate lengths, and exact
payload rows. Requests validate the five-field scope and bound cursors, select
at most 100 records, and return at most 512 KiB; payload selection occurs only
after candidate byte lengths pass those bounds. The adapter reports explicit
history gaps, empty/fully evicted epochs, continuation cursors, and redacted
metadata. Real tests cover the exact 512 KiB boundary, record cap, cursor edge
cases, page-level authorization/audit denials, and identified-reader snapshot
coherence across transactional append and delete/replace fixture mutations.
The isolated overlay is based on `05b5048` with only the three owned source
files changed (`/tmp/heph-reader-page-05b5048.lKhBun`): pinned Rust 1.88
formatting, affected-package strict Clippy, and workspace docs passed; the
reader, authz, and eight-test gateway PostgreSQL suites passed with migration
80 (`/tmp/heph-reader-page-{realpg,authz,gateway}-overlay.log`). Durable writer
integration, retention scheduling, RPC/protobuf exposure, project-cap loss
projection, and UI remain pending.

Live-guest crash recovery checkpoint (2026-09-19): on committed base
`95639da`, the isolated overlay
`/tmp/heph-persistent-crash-verify-20260919` ran the joined command
`HEPHAESTUS_APP_GATEWAY_SERVICE_E2E=1 scripts/run-gateway-libkrun-e2e.sh`.
After the warm pair and graceful restart proof, public Caddy
`/gateway/service/crash` returned the fixture's acknowledged 503, the guest
exited with code 42, and the daemon recorded redacted `unexpected_exit` data
for the old instance before reaching `cleaned`; its runtime, cgroup, and
materializer paths were absent. The daemon then made a new instance Ready and
active on the same immutable revision with a different startup ID, and two
public identity requests reached the replacement with count `2->3`. The run
recorded `persistent-service-crash ... exit_code=42`, passed 35 golden tests
with one ignored and the 8-test gateway-postgres resolver suite, emitted
`persistent-service-e2e=passed`, and confirmed
`daemon golden E2E passed; runtime and cgroup cleanup verified`. Final pinned
Rust 1.88 checks on the isolated overlay passed formatting, golden compilation,
strict targeted Clippy, and rustdoc. Logs:
`/tmp/heph-persistent-service-crash-e2e-20260919.log`,
`/tmp/heph-persistent-crash-compile-final-isolated-20260919.log`,
`/tmp/heph-persistent-crash-clippy-final-isolated-20260919.log`, and
`/tmp/heph-persistent-crash-doc-final-isolated-20260919.log`. This proves a
guest crash with the daemon alive and automatic replacement only; uncaught
daemon crash recovery, expired-boot recovery, cutover, revocation/drain, and
release UI remain pending.

Committed verification at `87b6ebe`: [CI run
35432737371](https://github.com/wimpheling/hephaestus/actions/runs/35432737371)
passes Rust/authorization, cooking applications, and live browser review. This
includes the guest-crash checkpoint; unclean daemon recovery remains incomplete.

Integration review in progress (2026-09-19): the cutover overlay passes the
failed-candidate case against real PostgreSQL: the active revision remains
Ready and the failed candidate is never provisioned. The pause/resume race now observes the real ownership adapter returning
Conflict before issuing the second pause, then verifies retry, drain, and
cleanup. Its isolated migration-78 PostgreSQL regression passes in
`/tmp/heph-cutover-stale-adapter-v3.log`. Third-revision admission is now verified through repeated scheduler scans
while both draining and physical destruction remain blocked; only after durable
cleanup does C become Ready. Evidence: `/tmp/heph-cutover-capacity-gated-v4.log`.
A timed-out exact target lookup also permits another gateway to retire, lease
renewal to continue, and Caddy to progress; evidence:
`/tmp/heph-cutover-fairness-v2.log`. Final isolated validation is complete on
an archive of `87b6ebe` with only the app cutover files overlaid: the full app
library suite passed 70 tests against real PostgreSQL migration 78 with the
worker role, strict Rust 1.88 app Clippy passed for all targets/features,
workspace formatting passed, and app rustdoc passed. Logs:
`/tmp/heph-cutover-final-app-tests.log`,
`/tmp/heph-cutover-final-app-clippy-overlay.log`,
`/tmp/heph-cutover-final-fmt-v2.log`, and
`/tmp/heph-cutover-final-app-doc.log`. This checkpoint excludes cleanup retry,
ambiguous-claim recovery, app-store integration, and real VM/Caddy cutover
proof.
The log-retention overlay passes mixed TTL/pressure deletion and project-low-
watermark regressions against migration 79. Retention remains incomplete:
the earlier one-chunk continuation regression has been restored, and metadata
GC eligibility, replay protection, and epoch-cap recovery have focused coverage.
Deterministic concurrent append serialization and final isolated checks remain
pending. The GC suite passes 11 real PostgreSQL tests in
`/tmp/heph-log-maintenance-gc-all-20260919b.log`.
External-daemon warm-service checkpoint (2026-09-19): the exact `87b6ebe`
overlay `/tmp/heph-external-warm-overlay-87b6` launched the Cargo-resolved
production `hephaestusd` binary, reached service `Ready`, published the real
Caddy route, served public requests with identity equality and request count
`2->3`, then handled SIGINT with durable `Cleaned` state and absent VM,
cgroup, and materializer paths. The daemon fix derives and creates
`runtime/exact-runs`, adds that host-owned root to the libkrun mount allowlist,
and reuses the same path for the run-runtime adapter. The wrapper's cleanup
check now permits only validated empty persistent namespaces and rejects files,
symlinks, or children. The joined run passed 35 golden tests and 8
gateway-postgres tests against migration 78, emitted
`persistent-service-external-warm-passed`, and ended with
`daemon golden E2E passed; runtime and cgroup cleanup verified`. Full log:
`/tmp/heph-external-warm-overlay-joined-final2-full-20260919.log`; daemon log:
`/tmp/heph-external-warm-overlay-real-daemon-final3.log`; pinned checks:
`/tmp/heph-external-warm-overlay-final-checks-20260919-{fmt,check,clippy,doc,shell}.log`.
This proves the external warm path and graceful shutdown only; unclean daemon
SIGKILL recovery, expired-boot recovery, later cutover, revocation/drain, and
release UI remain pending.

Durable service-log retention checkpoint (2026-09-19): migration 79, the
bounded gateway-edge maintenance port, and the worker-only PostgreSQL adapter
now have real PostgreSQL coverage for 24-hour server-clock TTL, 7/8 pressure
activation with 3/4 recovery, bounded payload and metadata deletion, durable
pressure continuation, exact quota/gateway/instance/epoch locking, watermark
replay protection, and cleaned-or-advanced-fence epoch eligibility. The
128-epoch capacity recovery and append/maintenance lock barrier pass in both
serialization orders. Exact-base isolated validation from `87b6ebe` is at
`/tmp/heph-retention-validation-87b6-501219`: 168 gateway-edge tests, the full
gateway-postgres suite (68 tests across its binaries), strict Rust 1.88
Clippy, formatting, and affected-crate rustdoc pass. Logs are
`/tmp/heph-retention-isolated-edge-tests-20260919.log`,
`/tmp/heph-retention-isolated-gateway-postgres-20260919.log`,
`/tmp/heph-retention-isolated-clippy-20260919.log`, and
`/tmp/heph-retention-isolated-doc-20260919.log`. Application writer
attachment, periodic maintenance scheduling, authorized readers/RPC, and UI
remain pending; this checkpoint does not claim those integrations.

Committed cutover verification at `98ba5c3`: [CI run
35435015740](https://github.com/wimpheling/hephaestus/actions/runs/35435015740)
passes Rust/authorization, cooking applications, and live browser review.

Committed retention verification at `32d857d`: [CI run
35435401903](https://github.com/wimpheling/hephaestus/actions/runs/35435401903)
passes Rust/authorization, cooking applications, and live browser review.

The bounded authorized service-log read contract is now defined in the edge
layer: exact project/gateway/revision/instance/fence scope, scope-bound
sequence cursors, 1--100 record pages with a fixed 512 KiB payload budget,
redacted debug output, and explicit epoch loss/retention metadata and history
incompleteness. The pinned Rust 1.88 isolated overlay at
`/tmp/heph-read-contract-b4c25f6.KIipk6` ran all 172 gateway-edge library
tests, strict all-target/all-feature Clippy, formatting, and rustdoc; logs are
`/tmp/heph-read-contract-b4c25f6-{edge-tests,clippy,fmt,doc}.log`. PostgreSQL
reader authorization, adapter queries, RPC/protobuf exposure, and project
usage loss visibility remain pending.

Committed production-daemon verification at `b4c25f6`: [CI run
35435872520](https://github.com/wimpheling/hephaestus/actions/runs/35435872520)
passes Rust/authorization, cooking applications, and live browser review.

Cleanup-retry checkpoint (2026-09-19): the daemon reconciliation loop now
retries retained supervisor cleanup through the parent polling loop with a
fair cursor, bounded backoff, and at most two in-flight cleanup retries. A
queued cleanup retry takes precedence over new startup admission without
stopping healthy services or Caddy reconciliation. The real PostgreSQL test
captures the original instance ID, fencing token, and VM ID, injects a first
destroy failure, and verifies that the same retained claim and VM are retried.
While the retry remains physically blocked, both the retained cleanup claim and
healthy replacement renew their heartbeats and two later target scans complete;
the next candidate is still unclaimed. Materializer cleanup and durable
`cleaned` state complete before that candidate is admitted. The final
migration-79 overlay at `/tmp/heph-retry-final-overlay.2LWC1V` passed the
focused regression and the full 71-test app library suite with real PostgreSQL
and the `hephaestus_worker` role. Pinned Rust 1.88 formatting, strict app
Clippy, and workspace rustdoc passed; logs are
`/tmp/heph-retry-b4-heartbeats-20260919.log`,
`/tmp/heph-retry-b4-full-final-app-20260919.log`,
`/tmp/heph-retry-b4-fmt-20260919.log`,
`/tmp/heph-retry-b4-clippy-20260919.log`, and
`/tmp/heph-retry-b4-doc-20260919.log`. Ambiguous-claim resolution,
expired-claim takeover, and release UI remain pending.

External-daemon unclean-recovery checkpoint (2026-09-19): the exact
`b4c25f6` base with only the golden fixture overlay at
`/tmp/heph-unclean-sigkill-source-20260919` ran
`HEPHAESTUS_APP_GATEWAY_SERVICE_E2E=1 HEPHAESTUS_APP_GATEWAY_SERVICE_EXTERNAL_E2E=1 HEPHAESTUS_APP_GATEWAY_SERVICE_EXTERNAL_CRASH_E2E=1 bash scripts/run-gateway-libkrun-e2e.sh` with the real external `hephaestusd`, PostgreSQL, NATS, libkrun, and Caddy path. The daemon reached `Ready`, served the warm pair, was terminated by SIGKILL, and the replacement daemon waited for the original DB-clock lease to expire, fenced and cleaned the old same-host instance and its VM/cgroup/materializer before admission. The proof records no post-crash instance row during the original live-lease window, validates candidate creation after both lease expiry and old `cleaned_at`, requires a fresh instance and startup identity, serves replacement requests with count `2->3`, then verifies final SIGINT cleanup. The joined run passed 35 golden tests and 8 gateway-postgres tests; evidence is `/tmp/heph-unclean-sigkill-joined-final-20260919.log`. Pinned Rust 1.88 checks passed: strict golden Clippy at `/tmp/heph-unclean-sigkill-clippy-final-20260919.log`, workspace formatting at `/tmp/heph-unclean-sigkill-fmt-final-20260919.log`, and workspace rustdoc at `/tmp/heph-unclean-sigkill-doc-final-20260919.log`. The shared and isolated golden source hashes are both `606f2e932a97090ec428ac475064fd6dbdca35bdae008ccef42ed657457bcb5e`. Current CI run `35436772998` for `e66bfc1` passes all three jobs. This proves external guest-service recovery after an unclean daemon exit only; broader boot reconciliation, cutover, revocation/drain, and release UI remain pending.


Authorized service-log metadata reader checkpoint (2026-09-19): migration 80
adds only application `SELECT(id, project_id)` on `gateways`; forced RLS,
explicit project/gateway `CanRead` checks, and the existing composite foreign
keys establish the exact instance/revision/project scope without exposing
`gateway_revisions` or project usage counters to the application role. The
PostgreSQL adapter returns empty metadata for a known instance without an
epoch, preserves readable historical fences, rejects future or mismatched
scopes, and commits authorization audits for denied and authorized-not-found
results. The exact `e66bfc1` archive overlay is
`/tmp/heph-reader-e66bfc1.mMYHbk`; pinned Rust 1.88 formatting, strict
gateway-postgres Clippy, and workspace rustdoc passed. Fresh application-role
metadata coverage passed with `max_migration=80`, and the eight-test
gateway-postgres regression suite passed; logs are
`/tmp/heph-reader-0080-isolated-{realpg,postgres-regression}.log`,
`/tmp/heph-reader-0080-isolated-{fmt3,clippy3,doc3}.log`. Payload pagination,
project-cap loss counters, RPC/protobuf exposure, and UI remain pending.

Committed unclean-restart checkpoint `1f5af08`: [CI run
35437400682](https://github.com/wimpheling/hephaestus/actions/runs/35437400682)
passes Rust/authorization, cooking applications, and live browser review.

Committed metadata-reader checkpoint `05b5048`: [CI run
35437687346](https://github.com/wimpheling/hephaestus/actions/runs/35437687346)
passes Rust/authorization, cooking applications, and live browser review.


Real VM/Caddy revision-cutover checkpoint (2026-09-19): the exact `05b5048`
archive with only `crates/hephaestus-app/tests/golden.rs` and
`crates/vm-libkrun/src/bin/heph-integration-check.rs` overlaid at
`/tmp/heph-cutover-verify-05b5048-r2` ran the joined
`HEPHAESTUS_APP_GATEWAY_SERVICE_E2E=1 HEPHAESTUS_APP_GATEWAY_SERVICE_EXTERNAL_E2E=1 HEPHAESTUS_APP_GATEWAY_SERVICE_CUTOVER_E2E=1 bash scripts/run-gateway-libkrun-e2e.sh`
command using the real external daemon, PostgreSQL, NATS, libkrun, and Caddy.
The fixture held an accepted A request for 20 seconds, then declared the
independently published B revision. A coherent database snapshot observed B
`Ready` and active while A was `Draining`; B served two public requests with
completed invocation bindings for its exact instance and fencing token while
A's hold remained pending. The hold then returned A's startup identity,
completed its accepted invocation, and removed A's VM runtime, cgroup, and
materializer before final B shutdown removed B's resources. The run recorded
`persistent-service-cutover-passed` with A instance
`bd98a2d9-cb67-4068-8a67-5c26e420d721`, B instance
`431ae4c2-b55d-4f40-a040-45f606363830`, A startup
`341-1789815614061755107`, and B startup
`340-1789815618007060204`. It passed 35 golden tests with one ignored and 8
gateway-postgres tests, ending with `daemon golden E2E passed; runtime and
cgroup cleanup verified`; full evidence is
`/tmp/heph-cutover-real-20260919-r2-v5.log`. The nullable publication actor
fixture fallback is validated against the gateway project's organization
membership. Pinned Rust 1.88 formatting, strict all-target/all-feature
Clippy for `hephaestus-app` and `vm-libkrun`, and workspace rustdoc passed;
logs are `/tmp/heph-cutover-r2-pinned-{fmt,app-clippy,vm-clippy,workspace-doc}.log`.
The shared and isolated owned-file hashes match (`golden.rs`
`1f53a09da63ac69c95122b06ca79618083a9dd9efa644e78ac20c703c9a8177b`, guest
helper `a1f85d814309569a739de8c65d6664ba3368db23eb2de4cb34651ba6a2ea8164`).
This proves real revision cutover and drain ordering only. Earlier checkpoints
cover guest and unclean-daemon crash recovery; same-process expired-claim
recovery, further revocation/adversarial paths, and release UI remain pending.

Real VM/Caddy operator-rollback checkpoint (2026-09-19): the exact `bd340cf`
archive with only `crates/hephaestus-app/tests/golden.rs` overlaid at
`/tmp/heph-rollback-verify-bd340cf` ran the joined
`HEPHAESTUS_APP_GATEWAY_SERVICE_E2E=1 HEPHAESTUS_APP_GATEWAY_SERVICE_EXTERNAL_E2E=1 HEPHAESTUS_APP_GATEWAY_SERVICE_CUTOVER_E2E=1 HEPHAESTUS_APP_GATEWAY_SERVICE_ROLLBACK_E2E=1 HEPHAESTUS_LIBKRUN_DIAGNOSTICS_DIR=/tmp/heph-rollback-diag-20260919-v1 bash scripts/run-gateway-libkrun-e2e.sh`
command with the real external daemon, PostgreSQL, NATS, libkrun, and Caddy.
After the forward A-to-B cutover, the fixture held an accepted B request,
restored the same immutable published A revision as the desired tip, and
observed a new A instance become `Ready` and active while B remained
`Draining`. The rollback A instance and B instance had distinct ownership and
startup identities; rollback A served two public requests bound to its exact
instance and fencing token while the B hold remained pending. B then completed
and reached durable `Cleaned` with its VM runtime, cgroup, and materializer
absent before final rollback-A shutdown and cleanup. The run recorded
`persistent-service-rollback-passed` and passed 35 golden tests with one
ignored plus 8 gateway-postgres tests with
`REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration=80`; the full log is
`/tmp/heph-rollback-real-20260919-v1.log`. The owned golden source hash is
`0eeda1c2ea8d02f96bfead8498db265cc6bb14e3c63f8207e157fbfa9b337fe3`.
Pinned Rust 1.88 formatting, strict app Clippy, and workspace rustdoc passed;
logs are `/tmp/heph-rollback-fmt-v1.log`,
`/tmp/heph-rollback-app-clippy-v3.log`, and
`/tmp/heph-rollback-workspace-doc-v1.log`. CI run `35439186418` for
`bd340cf` passed all three jobs. This proves operator rollback through the
real daemon/Caddy path; same-process expired-claim recovery, additional
adversarial cases, and release UI remain pending.

Additive service-log RPC contract checkpoint (2026-09-19): the gateway
protobuf now defines the exact project/gateway/revision/instance/fence scope,
stdout/stderr records with observed and stored timestamps, epoch loss and
retention metadata, and a bounded page request/response shape. The contract
documents 1--100 records, 64 KiB chunks, 512 KiB aggregate contents, and a
192-byte opaque cursor bound to all five scope fields. Raw contents are an
opt-in application-owned redaction responsibility; the platform does not
promise universal secret detection. The descriptor bytes allowlist contains
only the reviewed `GatewayServiceLogRecord.contents` field; no request-only
`sensitive` annotation was added. The exact `bd340cf` archive with only the
eight protocol-owned files overlaid is `/tmp/heph-proto-bd340cf.gCHmUn`.
Pinned Rust 1.88 descriptor policy tests passed 13/13, including the
100-record/512 KiB encoding budget test; strict `rpc-proto` Clippy,
formatting, rustdoc, Buf generation consistency, and protobuf breaking checks
passed. The RPC method, handler, cursor codec, application-role pool wiring,
and UI remain pending; this checkpoint exposes no service method yet. Logs are
`/tmp/heph-proto-bd340cf-{fmt,clippy,descriptor,doc}.log`,
`/tmp/heph-proto-check-generated.log`, and
`/tmp/heph-proto-check-breaking.log`.

Application-role pool checkpoint (2026-09-19):
`control_plane_postgres::connect_app` now selects `hephaestus_app` on every
new connection and fails closed if role selection fails. The isolated
`3ff1b3e` overlay is `/tmp/heph-connect-app-3ff1b3e.zvT82C`; the owned source
hashes are `lib.rs`
`a56d46de635cb4603e43ae229c8c5668e1e970da8fef65e576ceeaab8667907f` and
`tests/app_pool.rs`
`cf4b2a31e7d1a93de849829c96f1976e11b88f37c1d6d036098132c8ac1b7577`. The
real PostgreSQL test applied and observed migration 80, held two concurrent
connections reporting `current_user=hephaestus_app`, and verified SQLSTATE
42501 for an ungranted gateway column and the worker-only log usage table.
Pinned Rust 1.88 formatting, strict crate Clippy, and rustdoc passed. The
standalone real test log is
`/tmp/heph-connect-app-isolated-realpg.log`; checks are
`/tmp/heph-connect-app-isolated-{fmt,clippy,doc}.log`. The test skips only when
`HEPHAESTUS_POSTGRES_TEST_URL` is unset. RPC wiring remains pending.


Exact expired-claim CAS checkpoint (2026-09-19): the existing takeover port now receives the complete prior lease as its compare-and-swap witness. Under the existing gateway-to-instance locks and a fresh database clock, the PostgreSQL adapter compares the full instance/gateway/revision identity, prior owner host and UUID, fencing token, and deterministic VM ID before advancing exactly one fencing epoch and assigning the recovering owner. Boot recovery passes its inventory lease through this boundary; same-daemon expiry recovery remains supported without permitting a stale process to adopt a newer same-host epoch. The isolated overlay is based on `9bf2a615f103548b59f253e69c0ecf1f01d0cb74` at `/tmp/heph-expired-cas-verify-20260919`; the four owned source hashes match the shared worktree: `service_expired_claim_recovery.rs` `e72cd707b7fe5dc85905923972e0ef4d319c292be5fe831f8402474549c3c9e4`, `service_boot_recovery.rs` `24123f3b834c25396cb04002f032baf2f2f8c95beae796a885cdfa697c4e2848`, `service_ownership.rs` `ddbb64a228652dfcb2aab1538fed588223fe3a0020d30b35b12677d3db852d19`, and `expired_takeover.rs` `c1bb26f31c2e01317ebef430be42cf7af32af4e0b99b96b81b5d7e3ee880d37d`.

The isolated worker-role PostgreSQL takeover suite passed 6 tests with `REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration=80` in `/tmp/heph-expired-cas-isolated-realpg-20260919.log`; the full isolated gateway-edge library suite passed 172 tests in `/tmp/heph-expired-cas-isolated-edge-full-20260919.log`. Final pinned Rust 1.88 formatting and strict all-target/all-feature Clippy for `gateway-edge` and `gateway-postgres` passed in `/tmp/heph-expired-cas-fmt-final2-20260919.log` and `/tmp/heph-expired-cas-clippy-final2-20260919.log`. Supervisor expired-owned cleanup recovery and serialized lost-takeover resolution remain pending.


Daemon ambiguous-claim scheduling checkpoint (2026-09-19): the parent-owned
reconciliation loop now falls back from an ineligible cleanup retry to the
worker-role serialized claim-resolution store. It distinguishes a committed
lost acknowledgement from a confirmed absent claim, preserves the exact
instance/fencing witness, and keeps healthy service heartbeats and Caddy
progressing while a resolver is blocked behind the gateway row lock. The
blocked real PostgreSQL test observes the resolver backend itself blocked by
that lock, completes two target scans after selecting a replacement, and
proves replacement C has zero `claim_new` attempts, zero instances, and no
provisioning until B is durably `Cleaned`; C is admitted only afterward. The
focused blocked and lost-claim tests passed with migration 80 and the
`hephaestus_worker` role in `/tmp/heph-claim-blocked-realpg-v20.log` and
`/tmp/heph-claim-lost-final-v3.log`. The isolated overlay is
`/tmp/heph-claim-final-v1.Zpzw8p`, based on `d044142`; the three owned source
hashes match the shared worktree: app `lib.rs`
`e19dd9f82a70cbae9ab2ffcd8f8cf07f498ff520be5ba3d3e97410e2bc07ab41`, recovery
tests `46ee52bbd489df9ef3bdae4f782a88af3c055183f92f9ca5005004e5b9011d42`, and
`golden.rs`
`f71a7669befa8ae5de42612b63d941d785e7ea31f8b232562661ac0e39da2f20`.
The full app library passed 74/74 real-Postgres tests in
`/tmp/heph-claim-final-app-lib-v4.log`; the real PostgreSQL/NATS golden target
passed 35 tests with one ignored in `/tmp/heph-claim-final-golden-v1.log`.
Pinned Rust 1.88 strict app Clippy, workspace formatting, and workspace
rustdoc passed in `/tmp/heph-claim-final-clippy-v4.log`,
`/tmp/heph-claim-final-fmt-v4.log`, and `/tmp/heph-claim-final-doc-v1.log`.
The five golden restart call sites are boxed to keep the expanded daemon
future below the strict `large_futures` threshold. Claim ambiguity resolution,
cleanup retry ownership after process loss, and release UI remain separate
follow-up work.


Serialized exact-instance resolution checkpoint (2026-09-19): the expired-claim recovery port now exposes a read-only `resolve_exact_instance` operation for the complete instance/gateway/revision identity. The PostgreSQL adapter validates the identity, explicitly sets `READ COMMITTED`, acquires the same gateway `FOR UPDATE` barrier used by claim/takeover mutations, reads the exact row without filtering lifecycle state, and validates the decoded lease before returning it. `Cleaned` rows remain observable for ambiguous takeover or cleanup acknowledgements; wrong identities and missing rows resolve as exact absence, while malformed identity is rejected. The isolated overlay is based on `b6a38dd829739e098aff0b3580d0bf7fcd4013e5` at `/home/a/.cache/heph-exact-resolver-overlay-20260919`, with dedicated target `/home/a/.cache/heph-exact-resolver-target`. Owned source hashes match the overlay: `service_expired_claim_recovery.rs` `376469e921bbd9222bb4730a5c65c87d84b4e945cb9bf55e36c842508bb63ba3`, `service_boot_recovery.rs` `834ebf4b6b7e6b682a3f2a94ed37746f8adf32ed08ed7763d4657fbbf7f90a0`, `service_ownership.rs` `eafe493206a93c67ddf435c5b6dc73886feeea428baa2cd9cac470ff2e7d8697`, and `expired_takeover.rs` `3f9978d2eb3237447c3caa7e2da9e69abbe3a5913ed8f1b4d8487b6c16ab40fd`.

The worker-role PostgreSQL suite passed all 8 expired-takeover tests, including exact cleaned/absence lookup and two gateway-lock barrier phases. The test observed the resolver blocked by the holder PID before separately committing a staged takeover and rolling back a newer epoch; the final log with `REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration=80` is `/tmp/heph-exact-resolver-isolated-realpg-20260919.log`. The full isolated gateway-edge library passed 172 tests in `/tmp/heph-exact-resolver-isolated-edge-20260919.log`. Pinned Rust 1.88 formatting, strict all-target/all-feature Clippy for `gateway-edge` and `gateway-postgres`, and workspace rustdoc passed in `/tmp/heph-exact-resolver-isolated-fmt-20260919.log`, `/tmp/heph-exact-resolver-isolated-clippy-20260919.log`, and `/tmp/heph-exact-resolver-isolated-doc-20260919.log`. Supervisor use of exact resolution for expired owned cleanup and serialized lost-takeover handling remains pending.


Isolated-service teardown synchronization checkpoint (2026-09-19): the CI
failure at `9bf2a61` (`SQLSTATE 55006`, one session still using the generated
startup database) was not reproduced in the focused or concurrent 74-test
runs, so its underlying surviving-task cause remains unproven. The test
fixture now closes its control and worker pools, then waits up to two seconds
for `pg_stat_activity` to show no sessions for that exact generated database.
It records only PID, application name, state, and backend type on timeout; it
never force-terminates a session or hides a live task. The clean source archive
is based on `50ea7f`, at `/home/a/.cache/heph-teardown-src-50ea7f.8qqNgl`, with
dedicated target `/home/a/.cache/heph-teardown-target-50ea7f`. The shared and
verified recovery-test source hash is
`b5585f23c8de401c75c108ad37bd4e0e25e27ad1ba2648b0e17b7c290f473ed1`.
The focused revoked-active test passed with migration 80 and worker role in
`/tmp/heph-revoked-active-teardown-focused-v4.log`; the concurrent full app
suite passed 74/74 in `/tmp/heph-revoked-active-teardown-full-v1.log`.
Pinned Rust 1.88 formatting, strict app Clippy, and workspace rustdoc passed
in `/tmp/heph-revoked-active-teardown-fmt-v3.log`,
`/tmp/heph-revoked-active-teardown-clippy-v2.log`, and
`/tmp/heph-revoked-active-teardown-doc-v1.log`.

Native development documentation checkpoint (2026-09-19): the cooking-service
README and `docs/persistent-gateway-services.md` now describe the separate
native host smoke path and its boundary from managed VM/Caddy authority. With
Rust 1.88, the locked offline native suite passed 4 tests in
`/tmp/heph-cooking-native-doc-test-v2.log`, and the release build completed in
`/tmp/heph-cooking-native-doc-build-v2.log`. The exact documented `cargo run`
path served `ready` and `healthy`, returned the same `startup_id` from both
identity routes, and exited through process-group-scoped cleanup with port
8080 clear; evidence is `/tmp/heph-cooking-native-doc-run-v2.log`. The
debugger command was documented but not interactively exercised. The docs
record the concrete Connect operations (`SetDraftVersion`, `PublishRelease`,
`InstallReleaseGateways`, and `ConfigureGateway`) and state that the published
cooking-service build/publish/install/configure/readiness/Caddy/identity/
cleanup acceptance remains pending, while platform transport, bridge, Caddy,
and daemon lifecycle proofs are already covered by repository tests. Durable
service-log RPC/writer work remains pending.

Supervisor expired-owned cleanup recovery checkpoint (2026-09-19): the
parent-owned cleanup retry now accepts a serialized exact-instance recovery
port. It shares normal retry admission, falls back from stale/unavailable or
timed-out expected-fence takeover to the gateway-locked exact resolver,
validates the complete identity, deterministic VM ID, stable host, daemon
owner, fencing successor, lifecycle state, and lease shape, and adopts a
validated successor only after renewal succeeds before physical cleanup.
Validated ownership and all cleanup progress remain retained when renewal or
resolution fails; same-epoch `Cleaned` is accepted only with confirmed
physical and materializer cleanup. Focused tests cover successor expiry before
renewal, lost takeover and resolver acknowledgements across retries, absent,
foreign, newer, and rolled-back epochs, malformed/unrelated successors, and
premature `Cleaned` rows while preserving the original VM handle, pending
capacity, and zero orphan cleanup. The isolated Rust 1.88 overlay is based on
`bfd9dfda51bcfeabede24a562766b5dd7a061aae` at
`/home/a/.cache/heph-supervisor-recovery-overlay-b3dffac-v4`; the owned
`service_supervisor.rs` hash is
`f88741f7c0ce4e56950ae623dcd9235a2f33c4bf7504f1bb67edbd085fb8fff5`.
Gateway-edge passed 177 library tests, strict all-target/all-feature Clippy,
workspace formatting, and gateway-edge rustdoc. Logs are
`/home/a/.cache/heph-supervisor-recovery-overlay-b3dffac-v4/tests.log`,
`clippy.log`, `fmt.log`, and `doc.log`. Daemon/app scheduling integration,
same-process expired-lease recovery acceptance on real PostgreSQL, and the
external VM/Caddy acceptance remain subsequent work.

Final outbox diagnostics checkpoint (2026-09-19): shutdown flush failures now
emit a content-free snapshot containing the entry and remaining deadline
budget, pass count, active publisher and elapsed phase, last completed batch
counts, and whether the deadline expired before a pass or during a publisher.
Ordinary publisher errors are classified separately. The real-time timeout
regression uses a bounded 250 ms deadline because this crate does not enable
Tokio's test clock; it enters the publisher phase before awaiting the pending
operation. Four focused tests passed in
`/tmp/heph-outbox-overlay-focused-v2.log`. The isolated overlay is
`/tmp/heph-outbox-overlay-998ea`, based on `998ea998`, with dedicated target
`/home/a/.cache/heph-outbox-target-998ea`. Pinned Rust 1.88 strict app Clippy,
workspace formatting, and workspace rustdoc passed in
`/tmp/heph-outbox-overlay-clippy-v2.log`,
`/tmp/heph-outbox-overlay-fmt.log`, and
`/tmp/heph-outbox-overlay-doc.log`. The recurring populated PG/NATS shutdown
failure remains unproven and this checkpoint does not claim to fix it; a
separate reproduction is required.

Authenticated service-log RPC checkpoint (2026-09-19): the real libkrun/Caddy
golden path exercised `ListGatewayServiceLogs` through the generated Connect
client and the production application-role pool. It returned two fixture-seeded
stdout records across a signed scope-bound cursor, verified exact epoch
metadata, app-role access, tampered/cross-scope cursor rejection, outsider and
revoked-member denial, unauthenticated denial, and application-pool closure on
shutdown. The run passed 35 golden tests and 8 gateway-postgres tests with
`REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration=80`; the RPC marker was
`REAL_GATEWAY_SERVICE_LOG_RPC=1 app_role=hephaestus_app payload_cursor=1
denied=outsider+revoked+unauthenticated`. Evidence is
`/home/a/heph-gateway-rpc-isolated-v4.log`, from the exact `bfd9dfd` overlay
`/home/a/heph-gateway-rpc-overlay-bfd9dfd` and dedicated target
`/home/a/heph-gateway-rpc-target-bfd9dfd`. Final pinned Rust 1.88 formatting,
strict app/rpc-proto Clippy, default golden compilation, 77 app unit tests,
13 descriptor-policy tests, rustdoc, and generated-protobuf consistency passed;
the hash comparison is `/home/a/heph-gateway-rpc-sha256-manifest.txt`.
The rows are transactionally fixture-seeded for endpoint acceptance; production
guest-log writer attachment, durable retention, and UI remain pending.

Outbox diagnostics CI-lint correction checkpoint (2026-09-19): the three
focused diagnostics tests now explicitly drop their `MutexGuard` values after
assertions, satisfying `significant_drop_tightening` without a lint exception.
The isolated patch is exactly three lines on base `c129494` at
`/tmp/heph-outbox-ci-fix-6133b69`; the exact workspace command
`cargo +1.88.0 clippy --workspace --all-targets --all-features` passed in
`/tmp/heph-outbox-ci-fix-workspace-clippy.log`. The repository CI workflow
explicitly installs toolchain `1.88` before its Clippy step in
`.github/workflows/ci.yml`; the failed CI log records the same workspace
command but does not itself print a compiler version. Focused tests, package
Clippy, and formatting passed in `/tmp/heph-outbox-ci-fix-focused.log`,
`/tmp/heph-outbox-ci-fix-clippy.log`, and `/tmp/heph-outbox-ci-fix-fmt.log`.
The two subsequent populated PostgreSQL/NATS reruns passed without the
shutdown warning: `/tmp/heph-outbox-golden-repro-6133b69.log` (35 passed, one
ignored) and `/tmp/heph-outbox-full-app-lib-golden-6133b69.log` (76 app tests,
then 35 golden tests, one ignored). The recurring CI shutdown cause remains
unproven.

Daemon expired-owned cleanup integration checkpoint (2026-09-19): the daemon
now injects the worker-role PostgreSQL `GatewayServiceExpiredClaimRecovery`
adapter into its parent-owned cleanup retry loop, while retaining the existing
claim-resolution fallback and scheduling fairness. The unexpired retained
cleanup case remains covered, and the new real-PostgreSQL case holds the
instance row after the first destroy failure until the database clock confirms
lease expiry. Recovery then uses the same instance and deterministic VM, the
same host and daemon owner, and the expected fencing successor (`old + 1`);
the orphan path is unused. Capacity stays occupied while physical cleanup is
blocked and until durable `Cleaned`; after that, the next candidate's
`created_at` is asserted to be at or after the original row's non-null
`cleaned_at`. The focused isolated overlay
`/home/a/heph-app-recovery-overlay-20260919b` passed both worker-role real-PG
variants with `REAL_POSTGRES_CONNECTED_AND_MIGRATED=1` and formatting in
`/home/a/heph-app-recovery-overlay-closure-correct-env.log`. Earlier isolated
gates remain the accepted evidence: 80 app tests in
`/tmp/heph-app-overlay-appfull-20260919.log`, 15 recovery tests in
`/tmp/heph-app-overlay-fullpg-20260919.log`, workspace strict Clippy in
`/tmp/heph-app-recovery-overlay-f616-workspace-clippy-20260919.log`, and
workspace rustdoc in `/tmp/heph-app-recovery-overlay-f616-doc-20260919.log`.
The unrelated CI failure in `daemon_loop_restores_active_service_without_manual_start`
(`Ready` observed before the expected active revision; log
`/home/a/heph-ci-35444316773-failed.log`) is left for the next investigation.

Service declaration management-response checkpoint (2026-09-19): the
immutable `GatewayManagementRevision.service` projection is now exposed as an
optional `GatewayRevision.service` field 10. The typed response carries only
the loopback port, readiness path, health path, and disabled/application log
capture mode; stateless revisions omit it. `ConfigureGatewayRequest` remains
unchanged because the declaration is release-owned and immutable. Generated
Rust, descriptor, and Elixir artifacts pass consistency checks. Mapper tests
cover both log modes, valid stateless absence using the domain handler
contract constants, and an actual generated protobuf encode/decode roundtrip.
Descriptor policy tests cover exact field/message/enum shape and the absence
of sensitive annotations. The isolated Rust 1.88 overlay is based on
`f616772` at `/home/a/service-declaration-check.W6EoTl`; its final source
hashes are in `manifest.sha256`. The workspace all-target/all-feature Clippy
command passed on the isolated overlay; its captured command output and exit status are in
`workspace-clippy-final.log`. Formatting, generated consistency, mapper tests,
descriptor tests, and workspace rustdoc have captured logs at `fmt-final.log`,
`generated-final.log`, `mapper-final.log`, `descriptor-final.log`, and
`rustdoc-final.log` in that overlay. No mutable service configuration, runtime authority, credentials,
leases, VM identity, or readiness state is exposed. Real service management
and runtime wiring remain pending.

Restoration observation-race checkpoint (2026-09-19): the active-service
restoration test now waits for one SQL snapshot containing the exact gateway
and revision, an instance in `ready`, and that gateway's matching
`active_revision_id`; the predicate does not rely on fencing order across
instances. The assertion includes the fixture gateway and revision in its
bounded failure diagnostic. On clean `45a7f61` plus only this test overlay,
the focused worker-role PostgreSQL test passed with migration 80 in
`/home/a/heph-ready-promotion-race-focused-v3.log`; Rust 1.88 formatting and
all-target/all-feature app Clippy passed in
`/home/a/heph-ready-promotion-race-fmt-v3.log` and
`/home/a/heph-ready-promotion-race-clippy-v3.log`. The full recovery module
run recorded 14 passed and one unrelated teardown failure in
`/home/a/heph-ready-promotion-race-full-recovery-v2.log`: an idle PostgreSQL
client backend remained during isolated-database teardown. CI confirmed the
restoration test itself passed; the same run separately failed the expired
cleanup test because its global `DestroyGate` observed two different VM IDs
(`/home/a/heph-ci-35445311169-failed.log`). That gate synchronization remains
the next bounded test-only investigation and is not included here.

Expired-cleanup test-gate checkpoint (2026-09-19): the retained-cleanup
variants now arm the injected failure only after reading original A's durable
instance and deterministic VM ID. Retry notification is scoped to that VM;
all destroy IDs remain observable, and the test explicitly rejects an attempt
against healthy B while A is held. On clean `0a5ca54` plus only this test
overlay, both real worker-role PostgreSQL variants and the full 15-test
recovery module passed in `/home/a/heph-expired-gate-focused-v1.log` and
`/home/a/heph-expired-gate-full-recovery-v1.log`; Rust 1.88 formatting and
app all-target/all-feature Clippy passed in
`/home/a/heph-expired-gate-fmt-v2.log` and
`/home/a/heph-expired-gate-clippy-v1.log`. The final shared and overlay test
source hash is
`650d4bec4af1728184647751aa6c7f0acefbb72f448cc08f2815f63880932c91`.
This removes the test's global gate ambiguity and preserves the no-B-destroy
invariant, but does not establish the original CI differing-VM cause; that
cause remains open in `/home/a/heph-ci-35445311169-failed.log`.

Worker maintenance-project enumeration checkpoint (2026-09-19): the
worker-only log store now exposes UUID-only keyset pages capped at 128 rows.
Enumeration reads the unfiltered `gateway_service_log_project_usage` primary
key, so projects with no active service instance remain eligible for later
maintenance; the scheduler must retry failed projects fairly without blocking
the rest of a sweep. The real PostgreSQL test seeds a cleaned instance with a
retained epoch and storage-loss metadata, proves the 128+1 boundary and full
ordering without omissions, and verifies application-role SQLSTATE 42501
denial. The exact committed `cda7192` overlay plus four owned files is
`/home/a/service-enum-check-cda7192`; source hashes match the shared files.
The real run is captured in `postgres-enumeration-real-final.log` with
`REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration=80` and one test passed.
Affected-package Rust 1.88 Clippy, formatting, and the focused edge tests
passed in `clippy-final.log`, `fmt-final.log`, and `edge-tests-final.log`; the
prior real maintenance family is recorded in `postgres-tests-real.log`, and
the final enumeration-only rerun is `postgres-enumeration-real-final.log`. No migration,
scheduler, writer attachment, or RPC wiring is included; the earlier URL-unset
test run is not acceptance evidence.

Production service-log writer attachment checkpoint (2026-09-19): on base
`0b7a5e4a5fec0821c27265b582a479e1281bad85`, the application now constructs
`PostgresGatewayServiceLogStore` from the worker-role gateway pool and attaches
the existing bounded `GatewayServiceLogWriterConfig` through the production
reconciliation composition. The real PostgreSQL tests exercise synthetic VM
events through that production path, with per-test migrated databases and
`hephaestus_worker` markers. The final full recovery module passed 17 tests in
74.10 seconds, including the application-capture and disabled-capture cases;
the focused rerun passed both writer tests. Evidence is
`/home/a/heph-app-gateway-recovery-full-logwriter-v4.log` and
`/home/a/heph-app-log-writer-focused-v4.log`. The initial shared-fixture run
passed 15 and failed 2 readiness assertions; moving these two tests to isolated
databases resolved the fixture contamination. The application test verifies
one epoch and exact ordered stdout/stderr payloads, including the final stop
event, before durable cleanup. Rust 1.88 formatting, strict app all-target and
all-feature Clippy, and app rustdoc passed in
`/home/a/heph-app-service-log-fmt-v5.log`,
`/home/a/heph-app-service-log-clippy-v5.log`, and
`/home/a/heph-app-service-log-doc-v3.log`. The final shared hashes are
`lib.rs=a946132ed36c8b8aade93cc2649c1b72986c8b4c70221986895891725dc117d1`
and
`gateway_recovery_tests.rs=f25a513f4e8613f034d0874220278a2e6b6f674f2a30ea1a47947a5ddee3c79c`.
Default fake-provider event behavior remains unchanged, and no detached tasks
were introduced. This proves synthetic VM-event persistence through the
production reconciliation path; actual libkrun guest output through the
gateway RPC remains pending.

Failed-candidate real VM/Caddy checkpoint (2026-09-19): the clean `45a7f61`
golden overlay at `/dev/shm/heph-failed-candidate-current` now binds the
retained A fencing check to A's exact `(instance, gateway, revision)` identity,
while keeping B lifecycle evidence separate. The short-path libkrun run used
`/dev/shm/h` to keep Unix socket paths below the Linux limit and passed 35
 golden tests (one unrelated test ignored) in
`/home/a/heph-failed-candidate-e2e-short-v2.log`; the gateway PostgreSQL
sequence passed 8 tests with
`REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration=80`, seeded resolver
fixture, and query evidence in the same log. The acceptance observed A's
stable public startup identity across requests, B's durable
`failure_code=unexpected_exit`, `exit_code=42`, and no signal, zero B
invocations, unchanged gateway application-event count, exact B identity/fence,
and a same-SQL-snapshot retry streak/next-retry timestamp for B's failure and
cleaned state. There was no additional gateway event after B's desired
declaration. VM, cgroup, and materializer cleanup completed. Lifecycle polling
did not observe B in `Ready`; the ownership schema has no
historical `ready_at`, so this evidence does not claim an absolute historical
never-Ready proof. C-capacity retention, revocation/adversarial candidates,
and broader cutover scheduling remain separate acceptance work.
