# Identity PostgreSQL adapter

This crate owns PostgreSQL access for verified identity mapping, profile
refresh, and idempotent identity bootstrap. Its schema dependencies are the
root-owned `users`, `external_identities`, and `user_profiles` tables from
`migrations/0001_domain.sql`, plus the identity-profile events and occurrence
lookup defined by `migrations/0010_durable_application_events.sql`.

The adapter does not own schema changes. All schema DDL remains under the root
`migrations/` directory.
