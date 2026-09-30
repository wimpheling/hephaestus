# Purpose

`runtime-git-authority-postgres` stores the database record that connects one
runtime Git bearer to its exact session, repository, immutable scope, and
expiry. It supplies the PostgreSQL implementation used by Git HTTP before a
repository operation proceeds.

# Responsibilities

The repository inserts a verifier only when the generic runtime session and Git
snapshot are pending, current, and mutually consistent. Its authentication
function compares the presented verifier with the stored hash and resolves the
current scope, requested operation, expiry, and revocation state into
`AuthenticatedRuntimeGitAuthority`.

# When

Construct `PgRuntimeGitCredentialRepository` with the control-plane pool and
give it to `RuntimeGitCredentialIssuer` during exact runtime bootstrap. On
each Git request, pass the credential hash, repository ID, and parsed
`GitOperation` to `authenticate` before invoking Git capability enforcement.
