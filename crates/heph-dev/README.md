# Purpose

`hephaestus-dev` is the repository-local development environment supervisor.
Its `cargo dev` CLI builds selected components, starts the local stack, watches
Rust sources, manages PostgreSQL/NATS/Zot and workspace state, and runs the
repository's quality checks.

# Responsibilities

`DevContext` discovers the repository and local state roots, pinned service
images, ports, caches, logs, and registry paths. The CLI dispatches build,
run, doctor, status, logs, state, cache, platform-image, repository-image,
check, quality, and coverage workflows. The supervisor starts the local Zot
service and `scripts/run-local.sh`, forwards interrupts, and in watch mode
rebuilds and restarts the daemon and runtime after source changes.

The check commands compose architecture, protobuf, Rust, Phoenix, and UI
validation; `quality` runs the configured repository-wide gate. State and cache
commands operate on named resources so local development data can be inspected
or reinitialized deliberately.

# When

Run `cargo dev` with no subcommand to start the local supervisor, or use
`cargo dev --watch` for Rust rebuilds during development. Use `cargo dev build`
for selected components, `cargo dev check full` for focused checks, and
`cargo dev quality` before handing off a repository change.
