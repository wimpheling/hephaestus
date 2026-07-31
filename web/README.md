# Hephaestus Live Review

This Phoenix LiveView application is the browser control plane for the Rust
forge daemon. It does not own domain migrations or connect directly to domain
tables. Every protected read and command uses generated protobuf messages and
native gRPC stubs, with a short-lived mediator assertion scoped to the exact
RPC method.

Each product page consumes an authorized, scope-specific gRPC event stream,
resumes from its exact committed cursor, and re-fetches authoritative snapshots
after a typed event or mutation receipt is observed.

Configure `HEPHAESTUS_RPC_ENDPOINT`, `HEPHAESTUS_RPC_MEDIATOR_SECRET`, and the
browser OIDC settings documented in [`config/runtime.exs`](config/runtime.exs).
Then run:

```sh
mix setup
PHX_SERVER=true mix phx.server
```

The complete supported development path, including a local OIDC provider,
PostgreSQL, NATS, Rust daemon, deterministic VM result, and Chromium, is:

```sh
../scripts/run-ui-e2e.sh
```
