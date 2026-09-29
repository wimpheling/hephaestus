# Purpose

`pat-postgres` stores and serves Forge personal access tokens using the
hash-only domain record. It creates a token once, returns the bearer only in
the issuance result, lists safe metadata, and authenticates later Git requests
against PostgreSQL lifecycle and scope state.

# Responsibilities

`PostgresPersonalAccessTokenService` implements create, list, revoke, rotate,
and authenticate operations. Creation and rotation persist the token's
one-way verifier, owner, normalized Git scope, expiry, label, and request
identity; listing returns `PersonalAccessTokenMetadata` without bearer or
verifier material. Authentication reconstructs the domain record and applies
token, owner, lifecycle, operation, and repository checks before returning the
owner and token ID.

The service uses bounded lifetimes and typed domain errors, and it records last
use only after a successful authorization. SQL queries remain in this adapter,
while `pat-domain` owns token parsing, redaction, and scope semantics.

# When

Construct `PostgresPersonalAccessTokenService::new` with the Forge pool. Call
`create` for a user-facing issuance, `list` for management metadata,
`revoke` or `rotate` for lifecycle changes, and `authenticate` from the Git
transport before it asks Forge to read or write a repository.
