# Persistent gateway services

This guide uses `examples/cooking/cooking-service` as the smallest persistent
`http.service.v1` fixture. It has two useful modes:

* The native mode runs the ordinary Rust HTTP process on the host. It is quick
  and is suitable for request parsing, readiness, health, identity, debugger,
  and port inspection work.
* The managed mode publishes the source as an immutable release and lets the
  Hephaestus daemon materialize it in a network-disabled service VM. Caddy and
  the gateway authority are part of this path.

Native mode does not exercise VM isolation, Caddy routing, Connect
authorization, release materialization, or durable service lifecycle. A green
native smoke test therefore cannot stand in for the managed acceptance.

## Native host smoke

The fixture is a separate Cargo workspace. Use the repository's pinned Rust
toolchain and locked offline dependencies:

```sh
cargo +1.88.0 test \
  --manifest-path examples/cooking/cooking-service/Cargo.toml \
  --locked --offline
cargo +1.88.0 build \
  --manifest-path examples/cooking/cooking-service/Cargo.toml \
  --locked --offline --release
```

The service has no command-line configuration. It always binds
`127.0.0.1:8080`, prints a `cooking-service ready` line after binding, and
serves a bounded HTTP/1.0 or HTTP/1.1 request. Check the port before starting
it:

```sh
ss -H -ltnp '( sport = :8080 )'
```

The repository's local daemon also defaults to port 8080. Stop that stack, or
choose another daemon port with `HEPHAESTUS_LOCAL_DAEMON_PORT`; the cooking
fixture itself cannot be moved without changing its source. If the port is
occupied, the fixture exits with the bind error. Do not use `pkill` or kill a
PID discovered by a broad port search.

Start the fixture in one terminal and keep that terminal attached:

```sh
cargo +1.88.0 run \
  --manifest-path examples/cooking/cooking-service/Cargo.toml \
  --locked --offline --release
```

In another terminal, probe the process directly:

```sh
curl --fail --http1.1 http://127.0.0.1:8080/readyz
curl --fail --http1.1 http://127.0.0.1:8080/healthz
curl --fail --http1.1 http://127.0.0.1:8080/service
curl --fail --http1.1 http://127.0.0.1:8080/identity
curl --fail --http1.1 http://127.0.0.1:8080/service/identity
```

`/readyz` returns `ready` and `/healthz` returns `healthy`. The two identity
paths return JSON containing `pid` and `startup_id`; repeat either request and
confirm that the startup identity is unchanged. The public-looking
`/service` paths are only local process paths in this mode. They are not a
public gateway authority check.

Press Ctrl-C to stop a foreground process. For a scripted smoke test, launch
only the fixture process, retain its PID, and use a shell `trap` to send it
`TERM` and wait for it. This keeps cleanup scoped to the process started by
the test. If a prior run left a listener, identify its owning command with
`ss -ltnp` and resolve that owner explicitly before retrying.

There is no live reload for this separate workspace. `cargo dev run --watch`
rebuilds the Hephaestus daemon and watches Phoenix assets; it does not watch
or restart this cooking-service binary. After changing the fixture, stop it,
rerun the build or `cargo run` command, and repeat the probes.

## Debugging and inspection

Build a debuggable binary, then launch it under the installed Rust debugger:

```sh
cargo +1.88.0 build \
  --manifest-path examples/cooking/cooking-service/Cargo.toml \
  --locked --offline
rust-gdb --args \
  examples/cooking/cooking-service/target/debug/cooking-service
```

Inside GDB, `break main`, `run`, and `continue` are enough to reach the
listener. The process still owns the fixed 8080 port, so perform the `ss`
check first. While it is running, this read-only inspection shows the owning
PID and command without disturbing another listener:

```sh
ss -ltnp '( sport = :8080 )'
```

The service has a four-worker pool, an eight-entry connection queue, bounded
headers, and five-second read/write limits. It accepts GET requests only and
rejects request bodies, protocol upgrades, and unsupported paths. These limits
are fixture behavior, not configurable daemon policy.

The `rust-gdb` invocation above is the repository-supported debugger shape;
the bounded native smoke verification exercises the binary and HTTP probes,
not an interactive debugger session. The fixture's own readiness line and
diagnostic errors are written to its stdout/stderr. For the managed daemon,
follow the operational stream with:

```sh
cargo dev logs daemon --follow
```

Application service-log capture remains opt-in and application-owned so the
platform does not claim to redact arbitrary guest output. The separate guest
log acceptance mode records its own marker and durable stream evidence; a
missing entry in another mode is not proof that the guest did not run.

## Managed publish and install path

The source contract is in [`agent.toml`](../examples/cooking/cooking-service/agent.toml)
and [`heph.gateways.toml`](../examples/cooking/cooking-service/heph.gateways.toml).
The gateway declaration selects `http.service.v1`, loopback port 8080,
`/readyz` readiness, `/healthz` health, and the `/service`,
`/service/identity`, and diagnostic-only `/service/isolation` routes.
`build.sh` produces the `bin/cooking-service` executable from the isolated
builder and declares a network-disabled build and guest.

There is no standalone `publish-cooking-service` or `install-gateway` CLI.
The available managed operations are Connect RPCs:

1. Push the repository through the normal Git/build path, or use the existing
   build command where the caller already has a build request. The example
   helpers use the accepted push and wait for the isolated build.
2. Call `SetDraftVersion` and then `PublishRelease` on
   `hephaestus.release.v1.ReleaseService` for the successful immutable build.
3. Call `InstallReleaseGateways` on
   `hephaestus.gateway.v1.GatewayService` with that published `release_id`.
   The service resolves the release-owned manifest; callers do not submit a
   replacement manifest.
4. Read the installed gateway with `ListProjectGateways` or `GetGateway`,
   then call `ConfigureGateway` with the expected immutable revision and any
   typed parameters or secret selections required by the declaration.

The concrete operation names and request construction are in
[`examples/cooking/tests/builds.rs`](../examples/cooking/tests/builds.rs),
including `PublishRelease`, `InstallReleaseGateways`, and
`ConfigureGateway`. The ordinary cooking scenario also uses the shared
[`examples/cooking/run.sh`](../examples/cooking/run.sh) harness, which owns
disposable PostgreSQL, NATS, Caddy, guest filesystems, and cleanup.

The managed service is ready only after the daemon's service supervisor has
materialized the exact revision, started the service VM, observed the declared
readiness endpoint, and completed the durable readiness/promotion transition.
The public request should then go through the configured Caddy authority to
`/gateway/service` or `/gateway/service/identity`; these preserve the guest
declaration's `/service` and `/service/identity` paths under the gateway
authority. Probing the guest loopback address from the host is not an
equivalent test. A managed test should also assert that two identity requests
return the same `startup_id`, and that shutdown removes the service resources.

### Source-built Cooking service proof

The joined local harness has an opt-in proof for this exact source-built path.
It pushes the `examples/cooking/cooking-service` source through the production
Git/build flow, publishes the immutable release, installs its gateway
declaration, configures the release-owned revision, starts the service through
the daemon, sends requests through Caddy, checks the stable PID and
`startup_id`, and verifies cleanup of the VM, cgroup, and materializer paths.
The declaration's guest and build network profiles remain `disabled`; the
service has no workspace or state volume.

Prepare the local Cooking fixture and its pinned image/tool setup using the
repository's [Cooking CI runbook](gcp-cooking-ci.md). Source that local runner
profile without printing its values, then run from the repository root. The
profile supplies the immutable image and tool prerequisites; the command below
only selects the disposable project root, source root, Cargo settings, and
proof mode:

```sh
source /path/to/your/mvp05-cooking-runner.env
export HEPHAESTUS_LOCAL_ROOT="$PWD/.local/hephaestus"
export HEPHAESTUS_COOKING_SOURCE_ROOT="$PWD/examples/cooking"
export HEPHAESTUS_APP_COOKING_SERVICE_BUILD_PROOF=1
export HEPHAESTUS_CADDY_TEST_TLS=1
# Export this after sourcing the private profile so nested run scripts inherit
# the same cache. Use an absolute disposable shared path.
export CARGO_TARGET_DIR="/absolute/path/to/shared-service-target"
export CARGO_INCREMENTAL=0
export CARGO_BUILD_JOBS=2
unset HEPHAESTUS_POSTGRES_TEST_URL HEPHAESTUS_NATS_TEST_URL
unset HEPHAESTUS_APP_GATEWAY_SERVICE_E2E
unset HEPHAESTUS_APP_GATEWAY_SERVICE_EXTERNAL_E2E
unset HEPHAESTUS_APP_GATEWAY_SERVICE_REVOCATION_E2E
unset HEPHAESTUS_APP_GATEWAY_SERVICE_CUTOVER_E2E
unset HEPHAESTUS_APP_GATEWAY_SERVICE_CANDIDATE_CAPACITY_E2E
unset HEPHAESTUS_APP_GATEWAY_SERVICE_FAILED_CANDIDATE_E2E
unset HEPHAESTUS_APP_GATEWAY_SERVICE_ROLLBACK_E2E
unset HEPHAESTUS_APP_GATEWAY_SERVICE_LOG_RPC_E2E
unset HEPHAESTUS_APP_GATEWAY_SERVICE_LOG_GUEST_E2E
examples/cooking/run.sh
```

`examples/cooking/run.sh` supplies `HEPHAESTUS_APP_COOKING_E2E=1`,
`HEPHAESTUS_APP_COOKING_BUILD_PROOF=1`,
`HEPHAESTUS_COOKING_UPDATE_E2E=1`, and the browser/Caddy/libkrun fixture
boundary. The published service proof is an explicit opt-in and cannot be
combined with seeded gateway service, service-log RPC, guest-log, cutover,
rollback, or candidate-capacity modes. Leave PostgreSQL and NATS URLs unset so
the joined harness owns its disposable services. An absolute
`CARGO_TARGET_DIR` is recommended for a shared cache; the integration wrapper
resolves and exports relative values against the repository root and passes the
guest bootstrap from that resolved target.

The successful proof emits `REAL_COOKING_SERVICE_BUILD_PROOF=1`. The published
isolation extension additionally emits `REAL_COOKING_SERVICE_ISOLATION=1`.
It calls `/gateway/service/isolation` through Caddy with only the joined admin
and public listener port numbers. The guest performs a positive request to
its own loopback `/healthz`, then bounded blocked-connect probes to the Caddy
admin/public loopback ports, `169.254.169.254:80`, and TEST-NET
`192.0.2.1:80`. It reports booleans only and checks the source-correct
authority environment/path, broker socket, secret mount, and read-only
`/run/hephaestus/parameters.json` control surface containing `{}`. The host
also requests public `/config/` with ordinary and forged admin `Host` headers
and requires HTTP 404, then compares the service PID and `startup_id` before
and after the probe. This proves the published disposable guest's current
network and mount boundary.

When `HEPHAESTUS_CADDY_TEST_TLS=1` is paired with the published service proof,
the wrapper starts Caddy with its disposable internal CA and supplies the CA
PEM path to the golden client. The client trusts only that supplied fixture CA;
the Caddy `/load` template retains the internal issuer automation and the
HTTPS listener. The proof sends a normal metadata request and a second request
with forged `Host`, `Forwarded`, and `X-Forwarded-*` values through the HTTPS
IP URL. The sample returns only `host_matches_expected` and presence booleans
for the four forwarding-header names; both requests must preserve
`gateway.golden.invalid` and show no forwarding headers after the trusted
dispatcher. The stable identity request after the probes must still match the
pre-probe PID and `startup_id`. A successful TLS metadata proof emits
`REAL_COOKING_SERVICE_HTTPS_METADATA=1`.

The verified HTTPS metadata and isolation run passed 35 golden tests (one
ignored) and eight
PostgreSQL tests. Its log is
`/home/a/heph-published-service-https-real-vm-20260920-v2.log`, diagnostics
are under `/home/a/heph-published-service-https-diagnostics-20260920-v2`, and
phase timing is in
`/home/a/heph-published-service-https-phase-20260920-v2.json`. It emitted
`REAL_COOKING_SERVICE_HTTPS_METADATA=1`,
`REAL_COOKING_SERVICE_ISOLATION=1`, and
`REAL_COOKING_SERVICE_BUILD_PROOF=1`, and verified runtime, cgroup,
materializer, and VM cleanup. The initial TLS attempt is preserved in
`/home/a/heph-published-service-https-real-vm-20260920.log`; it reproduced a
Caddy TLS internal handshake error. The Caddy-only smoke evidence is in
`/home/a/heph-caddy-tls-reload-20260920-original.log` and
`/home/a/heph-caddy-tls-reload-20260920-fixed.log`; the correction adds the
explicit `apps.tls.certificates.automate` loader for `127.0.0.1` while
retaining the internal issuer policy. The seeded
gateway-service modes remain useful for their separate runtime scenarios, but
they do not prove this source-built publication path. This proof does not
claim that every persistent-service feature or the full service-log acceptance
surface is complete.

Platform-level private-service transport, host bridge, Caddy adapter, and
daemon lifecycle proofs are present in the repository's focused and real-VM
tests. The source-built Cooking service proof is now a verified local path; the
broader persistent-service and service-log acceptance surfaces remain
separate.

## Current limits and next acceptance work

The following remain deliberately separate from a native smoke and from the
source-built Cooking service proof:

* broader persistent-service lifecycle and cutover scenarios;
* service-log acceptance beyond the separate seeded and guest service-log
  proofs;
* deployment outside the disposable local Cooking fixture and its pinned
  runner prerequisites.

Use the existing real-stack harness after its required KVM, Podman, pinned
image, PostgreSQL, NATS, and Caddy prerequisites are available. Preserve its
diagnostic directory rather than replacing it with a manually started daemon.
For local daemon-only work, use `cargo dev doctor`, `cargo dev status`, and
`cargo dev logs daemon --follow`; state cleanup commands require the foreground
supervisor to be stopped.
