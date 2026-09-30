# Purpose

`rpc-proto` is the checked-in transport contract for Hephaestus protobuf
messages and Connect services. It supplies generated message views, service
traits and clients, and the descriptor set used by reflection and policy
checks.

# Responsibilities

The crate preserves the wire schema and descriptor metadata that clients and
servers share. Descriptor tests enforce package, field, streaming, sensitive
field, and compatibility policy. Transport handlers convert generated values
to inward-facing domain or application models at the boundary, keeping wire
names and optionality from becoming an accidental domain API. The descriptor
set also lets a server expose reflection without reconstructing schema from
handwritten code.

# When

Use this crate when implementing a Connect endpoint, client, or descriptor
policy check:

```rust
let descriptors = rpc_proto::descriptor_pool()?;
```

Keep authorization, persistence, and business validation in the receiving
application service after converting the generated request.
