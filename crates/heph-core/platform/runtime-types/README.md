# Purpose

`runtime-types` provides the stable UUID-backed identifiers shared by runtime,
run, volume, workspace, release, and event contracts. It gives separate crates
the same type for an agent instance, revision, attachment, release, run,
volume, lease, command, or event.

# Responsibilities

Each identifier is opaque, serializable, orderable, and printable while still
allowing explicit conversion to and from `Uuid`. The types prevent a run ID
from being passed where a volume or lease ID is required, and they keep durable
records and transport adapters consistent across context boundaries. The crate
contains identity values only; authorization and ownership checks remain with
the workflow that uses them.

# When

Use these types when creating or passing a durable runtime identity between
domain and adapter layers:

```rust
let run_id = runtime_types::RunId::new();
```

Persist and serialize the typed value rather than replacing it with an
unconstrained string.
