# Hephaestus application

`hephaestus-app` is the internal composition root and daemon lifecycle crate.
The `hephaestusd` binary is the runnable distribution entry point; this crate
is not a public SDK or a replacement for the core domain contracts.

## Responsibilities

- [`src/application/`](src/application/) builds validated configuration and
  resolves the database, NATS, storage, VM, gateway, identity, secret, image,
  forge, and run dependencies.
- [`src/composition/`](src/composition/) installs the concrete workers,
  consumers, routes, and cross-domain services selected for the local
  distribution.
- [`src/lib.rs`](src/lib.rs) owns the lifecycle: `build` validates and prepares
  dependencies, `start` binds listeners and starts supervised work, and
  `shutdown` stops admission, drains work, and closes resources.
- [`bootstrap/`](bootstrap/) contains deliberately trusted operator and test
  bootstrap boundaries. It seeds deterministic fixtures; it is not a second
  application API.

The application translates validated release and runtime data into provider
neutral contracts before handing work to workers. It rechecks live authority
at the launch boundary, starts durable outbox and command consumers, and
coordinates restart reconciliation for builds, runs, secrets, and gateway
services.

The direct dependency list is intentionally wider than the top-level facades:
the app must select concrete providers, persistence adapters, transports,
workers, and subcontext services. [`docs/application.md`](../../docs/application.md)
records that composition map and lifecycle evidence.

## Boundary

Domain meaning belongs in [`heph-core`](../heph-core/); concrete host behavior
belongs in [`heph-std`](../heph-std/). The app owns wiring, process lifecycle,
configuration, and trusted operational boundaries. It does not make guest code,
generated RPC types, or provider internals into shared domain contracts.

For the product direction and the remaining distribution work, see the
[roadmap](../../tasks/roadmap.md) and the [own-the-loop product definition](../../tasks/todo/distribution/define-own-the-loop-agent-platform.md).
