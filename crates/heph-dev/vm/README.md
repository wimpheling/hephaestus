# VM development tools

The `vm/` development directory contains reusable checks for implementations
of the core VM provider contract. It keeps provider behavior comparable across
the local fake, libkrun, and other runtime implementations.

The conformance crate supplies a provider harness and lifecycle suite covering
provisioning, shared concurrent starts and waits, typed invalid specifications,
idempotent stop/destroy behavior, identifier reuse, and caller-owned paths.
