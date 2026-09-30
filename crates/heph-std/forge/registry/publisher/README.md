# Purpose

`registry-publisher` safely publishes an administrator-selected OCI layout to
the configured private Zot registry and verifies the resulting immutable
manifest and supply-chain referrers. It is the host-side adapter used when a
publication intent has already selected the remote subject and local material.

# Responsibilities

`ControlledOciPublisher` validates the local OCI layout, hashes files, bounds
document sizes, derives the configured registry origin, and runs only the
approved command and HTTP operations. It verifies the remote manifest by
digest, platform descriptors, and required SBOM, provenance, scan, and
signature referrers before returning evidence. Trusted roots, executable paths,
registry authority, and publication subjects are configuration inputs rather
than caller-controlled strings; path and redirect checks prevent writes or
reads from escaping those roots.

# When

Use the publisher from a registry worker after loading a durable publication
intent and administrator-owned local layout:

```rust
let publisher = ControlledOciPublisher::new(configuration, command_runner);
```

Commit the returned verified digest and evidence through the registry adapter;
the publisher's successful process exit alone is not a usable publication.
