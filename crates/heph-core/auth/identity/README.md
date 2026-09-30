# Identity core contracts

[`identity-domain`](domain) owns provider-neutral authenticated identity,
tenant, request, and browser-session values. [`identity-application`](application)
owns SQL-free identity operations and persistence ports.

Concrete adapters are kept in the standard workspace:
[`identity-oidc`](../../../heph-std/identity/oidc) verifies external tokens,
[`identity-postgres`](../../../heph-std/identity/postgres) persists identity
and browser-session state, and
[`git-credential-hephaestus`](../../../heph-std/identity/git-credential)
implements the local Git credential-helper protocol.
