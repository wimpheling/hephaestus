# Event PostgreSQL adapter

This crate owns PostgreSQL access for the durable application-event context.
Its schema dependencies are the root-owned `application_events` table created
by `migrations/0010_durable_application_events.sql`, including the columns and
indexes used to identify the event committed for an idempotent mutation.

The adapter does not own schema changes. All schema DDL remains under the root
`migrations/` directory.
