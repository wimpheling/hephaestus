# Purpose

`registry-http` serves the Docker Distribution bearer-token exchange used by
the private OCI registry. It adapts an already-authenticated Hephaestus
identity and a live scope authorizer into the token response that Zot or a
registry client can use for one repository operation.

# Responsibilities

The service validates bounded query fields, parses the requested service and
scope, asks the authorizer for the actions currently allowed for that identity,
and signs a short-lived token containing only the authorized intersection. It
returns non-sensitive denial and availability responses and marks successful
responses `no-store`. The endpoint never accepts a registry-wide username or
password and does not mint a token when live authority cannot be evaluated.

# When

Mount the router behind the authentication layer that supplies
`AuthenticatedIdentity`, then configure the registry realm to point at it:

```rust
let router = RegistryTokenHttpService::new(issuer, scope_authorizer).router();
```

The registry client receives a scoped, expiring bearer token or a safe error;
the caller must request a new token when it expires.
