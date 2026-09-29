# Hephaestus core

`crates/heph-core/` is intended to contain provider-neutral domain, application,
port, and transport contracts. The current tree is transitional: it still
contains concrete PostgreSQL and transport adapters alongside those contracts.
The target boundary moves such adapters to `heph-std`; the core is not a second
daemon and does not select a host implementation.

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

The stable context facades are intended to be [`heph-forge`](forge/),
[`heph-runtime`](runtime/), [`heph-run`](runtime/run/), [`heph-secret`](auth/secret/), and
[`heph-build`](forge/build/). They expose provider-neutral APIs, although some
concrete adapters and cross-context services still remain in `heph-core` and
may be direct dependencies of the composition root during the migration.
[`heph-app`](../heph-app/) selects those implementations and assembles the
daemon.

Read the detailed operational contracts in [`docs/application.md`](../../docs/application.md),
[`docs/git-forge.md`](../../docs/git-forge.md),
[`docs/run-orchestration.md`](../../docs/run-orchestration.md), and
[`docs/releases-and-instances.md`](../../docs/releases-and-instances.md).
