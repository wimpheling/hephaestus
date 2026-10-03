# Purpose

`agent-config` parses and validates the repository's versioned `agent.toml`
contract. It turns declarative agent, build, guest, capability, gateway, UI,
parameter, secret-slot, workspace, state, and result settings into a normalized
value that release and run workflows can safely consume.

# Responsibilities

Parsing rejects malformed TOML, unknown fields, unsupported schema versions,
invalid paths, impossible resource limits, and inconsistent capability or
publication declarations. Successful input receives source and normalized
hashes so a release can identify the exact configuration it reviewed. The
parser keeps secret slots symbolic, requires explicit repository capability
ceilings for runtime-Git publication, and validates gateway and UI routes
without opening listeners or selecting tenant secrets. Repository image and
gateway manifests use the same bounded, hashable parsing boundary.

Authored `volume_slots` use validated paths, exact `read_only`/`read_write`
attachment modes, required/optional flags, and bounded minimum capacities. The
effective catalog adds enabled legacy state as required `state`, read-write at
`/var/lib/hephaestus`, with a one-byte compatibility minimum because old releases
declared no minimum. It validates duplicate keys and mount overlaps without
adding grants. Explicit volume keys cannot shadow generic capability keys.
Historical generic `state` key collisions stay parse-compatible but are
incompatible with recipe catalog resolution until explicitly reconciled.

Normalization sorts authored slots and omits empty catalogs. It never writes
the effective legacy declaration into source configuration, preserving old
normalized bytes and hashes. Named-volume persistence, instance binding, and
runtime mounting require separate integration; declaration validation does not
establish multi-volume execution. Operational entry points must reject explicit
slots until their exact bindings and attachment modes can be enforced.

# When

Use this crate when a receive or release workflow reads a repository manifest:

```rust
let parsed = agent_config::parse(source_bytes);
```

Proceed only when `parsed.config` is present and diagnostics are empty; retain
the hashes with the release or build identity.
