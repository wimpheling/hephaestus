# Identity PostgreSQL adapter

This crate moved from `crates/heph-core/identity/postgres` to
`crates/heph-std/identity/postgres` because it is a concrete database adapter.
The provider-neutral contracts remain in
[`identity-domain`](../../../heph-core/auth/identity/domain) and
[`identity-application`](../../../heph-core/auth/identity/application).

This crate owns PostgreSQL access for verified identity mapping, profile
refresh, and idempotent identity bootstrap. Its schema dependencies are the
root-owned `users`, `external_identities`, and `user_profiles` tables from
`migrations/0001_domain.sql`, plus the identity-profile events and occurrence
lookup defined by `migrations/0010_durable_application_events.sql`.

The adapter does not own schema changes. All schema DDL remains under the root
`migrations/` directory.
