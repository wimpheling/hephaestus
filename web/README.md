# Hephaestus Live Review

This Phoenix LiveView application is the browser control plane for the Rust
forge daemon. It does not own domain migrations or bypass PostgreSQL
authorization. Every protected read and command:

1. runs in a transaction;
2. installs `hephaestus.actor_id` and `hephaestus.request_id` locally;
3. relies on RLS and the generated Mélange permission functions.

Live database notifications carry only opaque wake-up identifiers. A LiveView
authorizes before subscribing and re-fetches protected data through RLS on
every wake-up and periodic reauthorization pass.

The Rust migration set must be applied before the web application starts.
Configure `DATABASE_URL`, the browser OIDC settings documented in
[`config/runtime.exs`](config/runtime.exs), and `HEPHAESTUS_ARTIFACT_ROOT`.
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
