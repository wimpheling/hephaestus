# Registry application tools

The `registry/` application directory contains trusted composition for
registry-facing workflows. Its release command joins Forge registry ownership
and publication records with the PostgreSQL store, controlled OCI publisher,
and narrowly scoped registry token issuer.

The directory sits above the core registry contracts and standard adapters: it
chooses the authority, storage, publisher, and command-line inputs needed for a
complete operator action, while the registry domain and provider crates retain
the lifecycle and transport rules.
