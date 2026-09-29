# Purpose

`registry-token` issues and verifies short-lived Docker Distribution bearer
tokens for one registry service and one or more requested repository scopes. It
turns a live authorization decision into a signed token that a registry client
can present for pull or push operations.

# Responsibilities

`ScopeRequest` and the registry scope types parse the requested service,
repository, and actions. `RegistryTokenIssuer` intersects that request with
the injected `AuthorizationDecision`, signs claims with a configured key, and
keeps lifetime, issuer, audience, key ID, and access claims bounded.

`RegistryTokenVerifier` selects a known rotation-window key, checks the
algorithm, issuer, audience, required claims, time limits, and normalized
access before returning claims. Bearer values and key material stay redacted
from errors and debug output.

# When

Use the issuer after the HTTP layer has authenticated the caller and resolved
current namespace ownership. Use the verifier at the registry edge before
serving the requested repository action.
