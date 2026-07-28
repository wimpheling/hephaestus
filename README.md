# Hephaestus

Hephaestus is a single-node Git forge and agent runtime proof of concept.

- [Git forge and agent ingestion](docs/git-forge.md)
- [Database-native identity and authorization](docs/authorization.md)
- [Daemon composition and lifecycle](docs/application.md)
- [Live review control plane](docs/live-review.md)
- [Run orchestration](docs/run-orchestration.md)
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
| [`run-orchestrator`](crates/run-orchestrator) | PostgreSQL, VM, volume, and JetStream coordination |
| [`forge-domain`](crates/forge-domain) | Project, repository, receive, and run-request domain values |
| [`agent-config`](crates/agent-config) | Versioned `agent.toml` parsing and validation |
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

The Rust workspace requires Rust 1.85 or newer.

```sh
cargo check --workspace
cargo clippy --workspace --all-targets --all-features
cargo test --workspace
```

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
