# Purpose

`secret-postgres` is the PostgreSQL adapter for secret commands, grants,
imports, immutable bindings, runtime leases, and mount provenance. It gives the
core secret application boundary durable authority checks and gives the run
orchestrator the database state needed to prepare and clean up mounts.

# Responsibilities

`SecretApplication` serves metadata queries inside actor transactions.
`SecretService` persists the command and grant lifecycle, while
`SecretRuntimeService` resolves exact raw or brokered leases, rechecks live
authority, decrypts through `EncryptedStore`, and records runtime outcomes.
`PostgresSecretMountMetadata` loads dispatch provenance, persists opaque mount
identities, checks active leases, and marks destroyed or reconciled mounts.

# When

Construct the PostgreSQL services with the secret pool, encrypted store, and
authorization provider, then call `initialize_manager` with the runtime
configuration. Use `SecretApplication` for authorized metadata views and let
the run orchestrator call the resulting manager around guest startup and
cleanup. Schema changes remain owned by the root migrations.
