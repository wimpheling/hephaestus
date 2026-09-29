# Hephaestus core

`crates/heph-core/` contains provider-neutral domain, application, port, and
transport contracts. The core is portable: concrete PostgreSQL, NATS, Git, VM,
filesystem, and other host adapters live in [`heph-std`](../heph-std/), while
trusted bootstrap binaries and daemon composition live in [`heph-app`](../heph-app/).
Core is not a second daemon and does not select a host implementation.

## Domain map

The current logical ownership map is:

| Domain | Owns | Current guide |
| --- | --- | --- |
| Forge | Projects, repositories, Git receive, builds, review, releases, and registry publication | [`forge/`](forge/) |
| Image | OCI image catalog and immutable image selection | [`image/`](image/) |
| Runtime | Runs, VM contracts, volumes, per-run workspaces, and mailbox mechanics | [`runtime/`](runtime/) |
| Platform | Gateway, routes, invocation, transport, and cross-domain mechanics | [`platform/`](platform/) |
| Auth | Identity, authorization, and secret contracts and policies | [`auth/`](auth/) |

The map describes contract ownership. Run orchestration and mailbox contracts
are under [`runtime/`](runtime/), gateway contracts are under
[`platform/gateway/`](platform/gateway/), identity and authorization contracts
are under [`auth/`](auth/), and secret contracts are under
[`auth/secret/`](auth/secret/). Their package metadata and the architecture
rules remain authoritative for current dependency boundaries.

## Composition boundaries

The stable context facades are [`heph-forge`](forge/),
[`heph-runtime`](runtime/), [`heph-run`](runtime/run/), [`heph-secret`](auth/secret/), and
[`heph-build`](forge/build/). They expose provider-neutral APIs. The
composition root in [`heph-app`](../heph-app/) selects implementations from
`heph-std`, installs cross-context adapters, and assembles the daemon.

Read the detailed operational contracts in [`docs/application.md`](../../docs/application.md),
[`docs/git-forge.md`](../../docs/git-forge.md),
[`docs/run-orchestration.md`](../../docs/run-orchestration.md), and
[`docs/releases-and-instances.md`](../../docs/releases-and-instances.md).
