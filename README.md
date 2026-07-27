# Hephaestus

Hephaestus is a secure, developer-focused Git forge and autonomous agent
runtime. It runs agents in isolated microVMs, manages repositories and pull
requests, enforces relationship-based access control, and streams real-time
execution telemetry to a live dashboard.

The project is powered by Rust, libkrun/libkrunfw, SpiceDB, NATS, Phoenix
LiveView, and Playwright.

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
