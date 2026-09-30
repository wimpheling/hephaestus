# Purpose

`runtime-git-authority` gives one runtime a narrowly scoped Git credential for
the repository operation it was authorized to perform. The credential is
issued during bootstrap, presented by Git HTTP, and resolved back to the exact
session, repository, scope, and expiry before any Git action proceeds.

# Responsibilities

`RuntimeGitCredential` turns random bytes into the canonical
`heph_git_v1_...` password accepted by Git HTTP and parses that form on the
way back. `RuntimeGitCredentialIssuer::issue` makes one session and generation
idempotent, while durable records retain only `RuntimeGitCredentialHash`.
Bearer bytes remain redacted and are wiped when the value is dropped.

On every Git request, `RuntimeGitCredentialRepository::authenticate` checks the
credential against the exact session, repository, requested operation,
immutable scope, expiry, and current revocation state. Only then does it
return `AuthenticatedRuntimeGitAuthority` to the transport.

# When

Issue the token during exact runtime bootstrap, give its protocol form to the
Git client, and authenticate each Git request at the transport boundary:

```rust
let issued = issuer.issue(session_id, generation, expires_at, now).await?;
let password = issued.credential.expose_token();
let authority = repository
    .authenticate(issued.credential.storage_hash(), repository_id, GitOperation::Read, now)
    .await?;
```
