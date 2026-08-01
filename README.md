# Hephaestus

Hephaestus is a single-node Git forge and agent runtime proof of concept.

- [Git forge and agent ingestion](docs/git-forge.md)
- [Database-native identity and authorization](docs/authorization.md)
- [Daemon composition and lifecycle](docs/application.md)
- [Live review control plane](docs/live-review.md)
- [Run orchestration](docs/run-orchestration.md)
- [Reusable releases and project agent instances](docs/releases-and-instances.md)
- [Secret delegation and runtime delivery](docs/secrets.md)
- [VM runtime](docs/vm-runtime.md)

Hephaestus is a secure, developer-focused Git forge and autonomous agent
runtime. It runs agents in isolated microVMs, manages repositories and pull
requests, enforces relationship-based access control, and streams real-time
execution telemetry to a live dashboard.

The current proof of concept is powered by Rust, PostgreSQL/Mélange,
libkrun/libkrunfw, and NATS JetStream.

## Workspace

The runtime starts with a provider-neutral VM interface and provider-specific
implementations:

| Crate | Purpose |
| --- | --- |
| [`vm-trait`](crates/vm-trait) | Shared abstractions for VM providers |
| [`vm-fake`](crates/vm-fake) | Deterministic provider for lifecycle and orchestration tests |
| [`vm-conformance`](crates/vm-conformance) | Reusable behavioral contract tests for every VM provider |
| [`vm-libkrun`](crates/vm-libkrun) | Local microVM provider backed by libkrun |
| [`runtime-types`](crates/runtime-types) | Stable identifiers shared by runtime domains |
| [`volume-trait`](crates/volume-trait) | Provider-neutral persistent-volume and lease contracts |
| [`volume-local`](crates/volume-local) | Single-host raw volumes with PostgreSQL metadata |
| [`run-domain`](crates/run-domain) | Durable run states and commands |
| [`run-orchestrator`](crates/run-orchestrator) | Provider-neutral VM, volume, repository, runtime-catalog, and JetStream coordination |
| [`run-postgres`](crates/run-postgres) | PostgreSQL run persistence and exact-runtime catalog adapter |
| [`run-runtime-local`](crates/run-runtime-local) | SQL-free local runtime artifact materialization and recovery |
| [`forge-domain`](crates/forge-domain) | Project, repository, receive, and run-request domain values |
| [`agent-config`](crates/agent-config) | Versioned `agent.toml` parsing and validation |
| [`release-domain`](crates/release-domain) | Immutable releases, project instances, revisions, attachments, updates, and typed policy |
| [`release-artifact-store`](crates/release-artifact-store) | One-way safe import into opaque immutable artifact storage |
| [`release-service`](crates/release-service) | Release publication, instance management, attachments, updates, and deferred triggers |
| [`build-orchestrator`](crates/build-orchestrator) | Exact-commit isolated builds and crash-safe draft release finalization |
| [`secret-domain`](crates/secret-domain) | Redacted secret ownership, delegation, binding, and lease contracts |
| [`secret-store`](crates/secret-store) | Authenticated encrypted immutable secret-version storage |
| [`secret-service`](crates/secret-service) | Grants, imports, bindings, exact dispatch resolution, rotation, revocation, and purge |
| [`secret-runtime`](crates/secret-runtime) | Ephemeral raw mounts and exact runtime secret authority |
| [`secret-broker`](crates/secret-broker) | Host-only semantic broker transport and bounded adapters |
| [`forge-service`](crates/forge-service) | Bare Git storage, PostgreSQL receive processing, and forge outbox |
| [`git-http`](crates/git-http) | Authorized streaming Git smart-HTTP transport |
| [`identity-domain`](crates/identity-domain) | Internal authenticated principal and tenant identifiers |
| [`identity-oidc`](crates/identity-oidc) | OIDC verification and identity mapping |
| [`authz-domain`](crates/authz-domain) | Typed provider-neutral authorization contract |
| [`authz-postgres`](crates/authz-postgres) | PostgreSQL/Mélange authorization and command auditing |
| [`workspace-domain`](crates/workspace-domain) | Provider-neutral exact-commit workspace and result contracts |
| [`workspace-local`](crates/workspace-local) | Safe local materialization, sealing, artifacts, and controlled Git result publication |
| [`review-domain`](crates/review-domain) | Durable review proposal and human-control commands |
| [`review-service`](crates/review-service) | Authorized cancel/retry/reject controls and CAS result-ref approval |
| [`hephaestus-app`](crates/hephaestus-app) | Production composition root and `hephaestusd` daemon |
| [`web`](web) | OIDC-authenticated Phoenix LiveView review control plane |
| [`e2e/playwright`](e2e/playwright) | Real browser golden path across the complete local stack |

## Documentation

- [VM runtime contract](docs/vm-runtime.md): lifecycle, guest bootstrap,
  parent/worker IPC, networking, image, disk, and mount contracts.
- [libkrun backend](docs/vm-libkrun.md): Fedora host contract, configuration,
  process isolation, cleanup, and integration-test requirements.
- [VM testing](docs/vm-testing.md): reusable provider conformance tests,
  backend component tests, and Fedora/KVM validation tiers.
- [Durable run orchestration](docs/run-orchestration.md): volume ownership,
  stale-lease recovery, database boundaries, and NATS subjects.
- [Reusable releases and instances](docs/releases-and-instances.md): isolated
  builds, immutable artifacts, project imports, revisions, attachments, exact
  runs, and stateful update hooks.
- [Secret delegation and delivery](docs/secrets.md): write-only encrypted
  values, explicit grants/imports, immutable bindings, raw mounts, brokered
  use, revocation, and audit.
- [Minimal Git forge core](docs/git-forge.md): canonical repository storage,
  smart HTTP, agent configuration, receive transactions, and run publication.
- [Daemon composition](docs/application.md): readiness, supervised tasks,
  graceful shutdown, configuration, and the golden end-to-end path.
- [Agent workspaces and results](docs/workspaces-and-results.md): immutable
  inputs, writable workspaces, sealing, safe import, artifacts, and controlled
  result refs.
- [Live review control plane](docs/live-review.md): browser OIDC, RLS-aware
  reads, re-authorized live updates, durable controls, and CAS approval.
- [Contributor instructions](AGENTS.md): repository-wide Rust quality and
  validation requirements.
- [Project TODO](TODO.md): deferred architectural decisions and completed
  runtime milestones.

## Development

The Rust workspace requires Rust 1.88 or newer.

```sh
cargo dev quality
```

For a persistent manual-smoke environment using real libkrun/KVM microVMs,
run:

```sh
cargo dev
```

`cargo dev` incrementally builds and starts PostgreSQL, NATS JetStream, a local
OIDC issuer, the Rust daemon, and Phoenix LiveView in the foreground. Press
Ctrl-C for a clean shutdown that retains state. Use `cargo dev --watch` to
rebuild changed Rust components and restart the daemon after successful builds;
Phoenix retains its native code and asset watchers.

The development CLI also exposes typed maintenance commands:

```sh
cargo dev doctor
cargo dev status
cargo dev build --daemon
cargo dev logs daemon --follow
cargo dev state list
cargo dev state reinit --postgresql --nats --fixtures
cargo dev state clean --all
cargo dev cache clean --rust
# Run the complete repository quality gate.
cargo dev quality
```

`cargo dev quality` is the single handoff command for repository changes. It
runs deterministic generated-code and Buf checks, architecture rules, Rust
formatting/Clippy/tests/docs, Phoenix checks, UI checks, and their focused
integration tests. Individual `cargo dev check <family>` commands remain useful
for fast iteration.

State selectors cover PostgreSQL, NATS, repositories, artifacts, agent
volumes, workspaces, secret keys, rootfs, fixtures, runtime files, and logs.
With no selectors, `state init`, `clean`, and `reinit` operate on every
resource. State mutation refuses to run while the foreground supervisor is
active. The host must satisfy the
[libkrun backend contract](docs/vm-libkrun.md).

For a disposable parallel state environment, set `HEPHAESTUS_LOCAL_ROOT`,
`HEPHAESTUS_LOCAL_NAMESPACE`, and (when the default port is occupied)
`HEPHAESTUS_LOCAL_POSTGRES_PORT`. The namespace isolates Podman containers and
volumes; the root and port settings isolate filesystem and PostgreSQL state.

On a prepared Fedora host, run the real KVM/libkrun smoke test without root:

```sh
scripts/run-libkrun-integration.sh
```

Run the complete daemon-level golden path through Git smart HTTP, PostgreSQL,
JetStream, the run orchestrator, and a real libkrun microVM with:

```sh
scripts/run-hephaestus-e2e.sh
```

Run the browser golden path through a local OIDC provider, Git smart HTTP,
PostgreSQL RLS, NATS JetStream, the deterministic VM fixture, Phoenix
LiveView, and Chromium with:

```sh
scripts/run-ui-e2e.sh
```

Build the metadata-only inspection and narrowly scoped recovery CLI with:

```sh
cargo build -p hephaestus-app --bin hephaestus-operator
target/debug/hephaestus-operator metrics <actor-uuid>
```

The CLI uses `HEPHAESTUS_DATABASE_URL`, reauthorizes every operation, and
audits mutating recovery commands. See the
[release/instance operator runbook](docs/releases-and-instances.md#local-workflow-and-operator-recovery)
and [secret operator runbook](docs/secrets.md#operator-runbook) before using
recovery or key-management controls.
