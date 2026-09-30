# Purpose

`hephaestus-app` is the production composition root and daemon lifecycle for
the Hephaestus distribution. It turns validated configuration into a running
installation with concrete database, NATS, Git, VM, gateway, identity,
secret, image, Forge, and runtime providers.

# Responsibilities

`HephaestusApp` builds the dependency graph, validates startup configuration,
installs RPC routes and background workers, starts supervised listeners and
consumers, and shuts them down in an orderly sequence. The composition layer
converts generated transport values into core contracts, rechecks live
authority at launch boundaries, and starts reconciliation for outboxes, builds,
runs, secrets, and gateway services.

The `hephaestusd` binary is the distribution entry point. Trusted bootstrap
utilities live under `bootstrap/`; they seed operator or end-to-end fixtures
through explicit boundaries. Product meaning remains in `heph-core`, while
host behavior is selected from `heph-std` during composition.

# When

Use this crate when assembling or running the local Hephaestus distribution.
Call `HephaestusApp::build` with `AppConfig` during startup, then call
`start` to bind listeners and begin supervised work; call `shutdown` when the
process receives its termination signal. The normal binary is
`hephaestusd`.
