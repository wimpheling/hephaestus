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

Application service-log Connect operations and the daemon's durable service
log writer are still being completed. Do not treat a missing service-log entry
as proof that the guest did not run; use the declared readiness/health probes,
the managed request result, and the daemon's existing operational log until
that writer is attached. Any application log capture must remain opt-in and
application-owned so the platform does not claim to redact arbitrary guest
output.

## Managed publish and install path

The source contract is in [`agent.toml`](../examples/cooking/cooking-service/agent.toml)
and [`heph.gateways.toml`](../examples/cooking/cooking-service/heph.gateways.toml).
The gateway declaration selects `http.service.v1`, loopback port 8080,
`/readyz` readiness, `/healthz` health, and the `/service` and
`/service/identity` routes. `build.sh` produces the `bin/cooking-service`
executable from the isolated builder and declares a network-disabled build and
guest.

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
`/service` or `/service/identity`; probing the guest loopback address from the
host is not an equivalent test. A managed test should also assert that two
identity requests return the same `startup_id`, and that shutdown removes the
service resources.

Platform-level private-service transport, host bridge, Caddy adapter, and
daemon lifecycle proofs are present in the repository's focused and real-VM
tests. The published cooking-service workflow still needs its own end-to-end
acceptance across build, publish, install, configure, readiness, Caddy
request, identity, and cleanup. The existing broad cooking acceptance covers
the ordinary cooking application path and does not by itself close that
published persistent-service acceptance.

## Current limits and next acceptance work

The following are deliberately separate from a native smoke:

* run the cooking source through its isolated build and release publication;
* install its `http.service.v1` declaration and configure the exact revision;
* start it through the daemon's managed service path rather than manually
  running the binary;
* let the service supervisor poll the private guest readiness and health
  paths, then make Caddy requests only to the declared `/service` and
  `/service/identity` routes;
* verify same-process identity, disabled guest ingress, no runtime bearer, and
  complete VM, materialization, registry, and durable-row cleanup.

Use the existing real-stack harness after its required KVM, Podman, pinned
image, PostgreSQL, NATS, and Caddy prerequisites are available. Preserve its
diagnostic directory rather than replacing it with a manually started daemon.
For local daemon-only work, use `cargo dev doctor`, `cargo dev status`, and
`cargo dev logs daemon --follow`; state cleanup commands require the foreground
supervisor to be stopped.
