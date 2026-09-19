# Cooking service

This is a small ordinary HTTP release used to prove the persistent
`http.service.v1` gateway path. It listens only on `127.0.0.1:8080`, has no
workspace or state volume, and receives no secret or runtime-authority input.
The service accepts bounded `GET` requests and closes each response.

The private readiness and health paths are `/readyz` and `/healthz`. The
process returns simple text at `/` and `/service`; the gateway publishes
`/service`, while `/identity` and `/service/identity` return a stable PID and
startup identity for the lifetime of one process. The server
uses a fixed worker pool, bounded connection queue, header limits, and five
second read/write timeouts. It intentionally has no crash, admin, WebSocket,
upgrade, trailer, or request-body behavior.

Run the native tests and build with the repository toolchain:

```sh
cargo test --manifest-path examples/cooking/cooking-service/Cargo.toml
cargo build --manifest-path examples/cooking/cooking-service/Cargo.toml \
  --locked --offline --release
```

For a local smoke request, run the binary in one terminal and use `curl` from
another:

```sh
cargo run --manifest-path examples/cooking/cooking-service/Cargo.toml --release
curl --http1.1 http://127.0.0.1:8080/service
curl --http1.1 http://127.0.0.1:8080/service/identity
```

The production build is the repository's normal Git/build/release workflow:
push this source with `agent.toml` and `heph.gateways.toml`, wait for the
isolated build, set a draft version, publish the release, and call
`InstallReleaseGateways`. The current repository has those operations in its
Connect APIs and cooking acceptance helpers; it does not provide a standalone
publish/install CLI. Persistent-service Caddy forwarding and managed lifecycle
acceptance remain pending platform work.
