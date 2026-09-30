# Purpose

`run-runtime-local` materializes the exact immutable runtime tree used by a
run or a persistent gateway service. It reads release artifact metadata through
the run catalog, copies verified objects into a fresh administrator-owned tree,
and returns read-only VM mounts for `/release` and `/run/hephaestus`.

# Responsibilities

Initialization canonicalizes separate runtime and release-object roots and
rejects overlap or unsafe permissions. Run materialization bounds artifact
count and bytes, rehashes each opaque object while copying, writes parameters,
mailbox input, and host context, seals the trees read-only, and activates them
atomically. Update runs receive a sealed previous release as well. Gateway
materialization and persistent service trees use distinct identities and cleanup
paths; recovery validates identity metadata and removes only owned staging or
active trees. These checks keep repository paths, mutable object-store files,
and untrusted guest writes from becoming runtime input.

# When

Initialize the manager during host composition and install it as the run
runtime manager:

```rust
let manager = LocalRunRuntimeManager::initialize(catalog, config)?;
```

The caller receives `PreparedRunRuntime` mounts or a redacted materialization
error; after VM destruction it must invoke the matching cleanup path.
