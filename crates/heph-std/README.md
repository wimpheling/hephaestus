# Hephaestus standard implementations

`crates/heph-std/` is the home for concrete providers and transports for the
provider-neutral contracts in [`heph-core`](../heph-core/). It supplies the
PostgreSQL, NATS, Git, VM, filesystem, and other host integrations used by the
current single-node distribution. Trusted bootstrap binaries and cross-context
startup composition live in [`heph-app`](../heph-app/); the core contracts stay
portable.

## Implementations

- [`runtime/`](runtime/) provides the libkrun and fake VM providers, local
  persistent volumes, and local exact-commit workspaces.
- [`run/`](run/) materializes immutable releases and host context in per-run
  filesystem roots and performs recovery without SQL.
- [`forge/`](forge/) provides canonical bare-Git storage, PostgreSQL metadata
  and receive processing, NATS outbox publication, local OCI preparation
  workers, smart HTTP, review Git publication, and the notification, publisher,
  and Zot registry integrations.
- [`gateway/`](gateway/) reconciles the private Caddy edge and dispatches
  bounded HTTP requests to released gateway services.
- [`identity/`](identity/) contains concrete identity adapters:
  [`identity-oidc`](identity/oidc/) verifies external tokens,
  [`identity-postgres`](identity/postgres/) persists identity and
  browser-session state, and
  [`git-credential-hephaestus`](identity/git-credential/) implements the local
  Git credential-helper protocol.
- [`authorization/`](authorization/) protects the host-to-VM runtime handoff.
- [`secret/`](secret/) supplies the bounded guest-to-host broker and ephemeral
  secret mounts.

The fake VM provider and other deterministic adapters are test implementations
of the same contracts. The standard crates do not redefine project, release,
image, route, or run semantics; they implement the ports and preserve the
core's cleanup, authority, and provenance rules.

## Boundary

`heph-std` crates are leaves. They may use libkrun, Caddy, local filesystems,
host tools, or transport libraries, but they do not assemble the daemon or own
cross-context startup. [`heph-app`](../heph-app/) selects and composes them;
the root migrations and declared PostgreSQL adapters remain the persistence
boundary.

See [`docs/application.md`](../../docs/application.md) for the concrete
dependency map and [`docs/vm-runtime.md`](../../docs/vm-runtime.md) for the VM
provider contract.
