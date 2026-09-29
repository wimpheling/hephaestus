# Purpose

`authz-domain` gives an authenticated request one consistent way to say who is
acting, which resource is involved, and what operation is requested. A
provider can then return an `Allow` or `Deny` decision that the transport can
apply without interpreting provider-specific values.

# Responsibilities

The model gives users, exact runs, and agent instances a stable subject form,
and gives repositories, releases, runs, and secrets a stable resource form.
Canonical permission and object names survive storage and transport unchanged;
unknown or malformed names become typed `AuthzError` values instead of being
interpreted loosely.

At a Git request boundary, an adapter uses `GitRepositoryAuthorizer` with the
authenticated identity, repository ID, and requested read or write operation.
The configured provider evaluates its policy; the standard provider applies
current membership and revocation rules before returning the `Allow` or `Deny`
result the transport enforces.

# When

Use this crate at a request boundary before starting a Git operation or other
protected action. A transport can turn a denied decision into its normal
unauthorized response:

```rust
let decision = authorizer
    .authorize_git(repository_id, GitRepositoryOperation::Read, &identity)
    .await?;
if !decision.is_allowed() { /* reject the transport request */ }
```
