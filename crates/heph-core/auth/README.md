# Authentication core

This directory owns the provider-neutral authentication, identity,
authorization, and secret contracts used by the rest of the core workspace.
Auth is the logical owner for these concerns even while authorization and
secret retain their current physical roots at [`../authorization/`](../authorization/)
and [`../secret/`](../secret/).

## Contracts and boundaries

- [`identity/`](identity/) defines authenticated identity, tenant, request, and
  browser-session values and the SQL-free identity operations that use them.
- [`../authorization/`](../authorization/) defines capability, runtime
  authority, and audit contracts for deciding whether work may proceed.
- [`../secret/`](../secret/) defines secret references, storage, brokered
  egress, and the bounded guest-to-host secret boundary.

Concrete providers live under
[`heph-std/identity`](../../heph-std/identity):
[`identity-oidc`](../../heph-std/identity/oidc),
[`identity-postgres`](../../heph-std/identity/postgres), and
[`git-credential-hephaestus`](../../heph-std/identity/git-credential).
Authorization providers live under
[`heph-std/authorization`](../../heph-std/authorization), and secret providers
live under [`heph-std/secret`](../../heph-std/secret). See the
[identity guide](identity/README.md) for the identity contract boundary and
the [authorization](../../../docs/authorization.md) and
[secret](../../../docs/secrets.md) guides for their operational boundaries.
