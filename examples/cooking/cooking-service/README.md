# Cooking service

This is a small ordinary HTTP release used to prove the persistent
`http.service.v1` gateway path. It listens only on `127.0.0.1:8080`, has no
workspace or state volume, and receives no secret or runtime-authority input.
The service accepts bounded `GET` requests and closes each response.

The private readiness and health paths are `/readyz` and `/healthz`. The
process returns simple text at `/` and `/service`; the native process exposes
those paths directly, while the joined managed gateway preserves them under
`/gateway/service`. The `/identity` and `/service/identity` paths return a
stable PID and startup identity for the lifetime of one process. The server
uses a fixed worker pool, bounded connection queue, header limits, and five
second read/write timeouts. It intentionally has no crash, admin, WebSocket,
upgrade, trailer, or request-body behavior.

Run the native tests and build with the repository toolchain:

```sh
cargo +1.88.0 test --manifest-path examples/cooking/cooking-service/Cargo.toml \
  --locked --offline
cargo +1.88.0 build --manifest-path examples/cooking/cooking-service/Cargo.toml \
  --locked --offline --release
```

The host CI and repository quality gate use these locked, offline Cargo
commands; they do not invoke the guest-only `build.sh` toolchain. The release
artifact is attached for deployment with `Manual` `trigger_policy`. The
top-level `[triggers]` section is currently parser-only and does not schedule
runs; `[build].triggers` is used when matching build requests.

For a local smoke request, run the binary in one terminal and use `curl` from
another:

```sh
cargo +1.88.0 run --manifest-path examples/cooking/cooking-service/Cargo.toml \
  --locked --offline --release
curl --http1.1 http://127.0.0.1:8080/service
curl --http1.1 http://127.0.0.1:8080/service/identity
```

For the complete native and managed workflow, including readiness, health,
identity, debugger, port, cleanup, and live-reload boundaries, see
[`docs/persistent-gateway-services.md`](../../../docs/persistent-gateway-services.md).

The production build is the repository's normal Git/build/release workflow:
push this source with `agent.toml` and `heph.gateways.toml`, wait for the
isolated build, set a draft version, publish the release, and call
`InstallReleaseGateways`. The current repository has those operations in its
Connect APIs and cooking acceptance helpers; it does not provide a standalone
publish/install CLI. The joined local harness can run the complete source-built
proof by setting `HEPHAESTUS_APP_COOKING_SERVICE_BUILD_PROOF=1` and invoking
`examples/cooking/run.sh` after the local Cooking profile and pinned fixture
prerequisites are prepared. Keep PostgreSQL and NATS URLs unset so the harness
owns disposable services, and use the shared `CARGO_TARGET_DIR` supported by
the integration wrapper. The proof builds and publishes this source, installs
and configures the release-owned gateway, serves `/gateway/service` and
`/gateway/service/identity` through Caddy, verifies stable process identity and
the asserted disabled runtime network contract, and checks VM, cgroup, and
materializer cleanup. It does not use a guest egress probe.
The seeded gateway-service modes cover separate runtime scenarios and do not
replace this source-built publication proof; neither mode claims overall
persistent-service completion.

The exact opt-in command and mutually exclusive seeded flags are in
[`docs/persistent-gateway-services.md`](../../../docs/persistent-gateway-services.md#source-built-cooking-service-proof).
