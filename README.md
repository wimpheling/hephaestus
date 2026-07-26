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
| [`vm-libkrun`](crates/vm-libkrun) | Local microVM provider backed by libkrun |

## Documentation

- [VM runtime contract](docs/vm-runtime.md): lifecycle, guest bootstrap,
  parent/worker IPC, networking, image, disk, and mount contracts.
- [libkrun backend](docs/vm-libkrun.md): Fedora host contract, configuration,
  process isolation, cleanup, and integration-test requirements.
- [Contributor instructions](AGENTS.md): repository-wide Rust quality and
  validation requirements.

## Development

The Rust workspace requires Rust 1.85 or newer.

```sh
cargo check --workspace
cargo clippy --workspace --all-targets --all-features
cargo test --workspace
```
