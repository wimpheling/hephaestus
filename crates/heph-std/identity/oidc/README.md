# Purpose

`identity-oidc` verifies OIDC JWTs at the external identity boundary and
returns the provider-neutral verified identity consumed by core identity
application code.

# Responsibilities

`OidcVerifier` is configured with an issuer, audience, algorithm, and decoding
key already obtained from trusted JWKS configuration. It validates the
signature, issuer, audience, required `exp`/`iss`/`sub`/`aud` claims, expiry,
and an expected nonce when an interactive flow supplied one. The separate
state helper compares the authorization response to the server-side session
value.

# When

Construct one verifier per trusted issuer and pass it to authentication
middleware as an `ExternalIdentityVerifier`. Supply the authorization-flow
nonce to `verify`; pass `None` only for bearer-token flows that did not create a
nonce, then map the returned issuer and subject through the identity provider.
