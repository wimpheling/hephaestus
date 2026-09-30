# Purpose

`identity-postgres` persists the mapping from a verified external issuer and
subject to an active Heph user, and implements the browser-session application
ports. It gives the authentication boundary durable idempotency and session
state while keeping provider-neutral identity values in the core crates.

# Responsibilities

`PostgresIdentityStore` resolves active mappings in a transaction, derives the
actor-bound idempotency identity, refreshes validated profile data, and records
the identity event used for replay. Its bootstrap operation creates trusted
user and external-identity mappings. `PostgresBrowserSessionStore` writes
sessions through a worker pool, verifies them through a separate application
pool and security-definer function, and stores only SID and identity-binding
digests.

# When

Construct `PostgresIdentityStore` with the identity pool and use it as both the
verified mapper and idempotent resolver. Construct
`PostgresBrowserSessionStore` with separate worker and application pools, then
call its create, authenticate, and revoke operations at the browser-session
boundary. The adapter expects the root migrations to have created its tables
and database functions.
