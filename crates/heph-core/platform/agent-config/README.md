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

# When

Use this crate when a receive or release workflow reads a repository manifest:

```rust
let parsed = agent_config::parse(source_bytes);
```

Proceed only when `parsed.config` is present and diagnostics are empty; retain
the hashes with the release or build identity.
