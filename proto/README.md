# Shared application protocol

`proto/hephaestus/*/v1` is the single owned source for the Rust Connect/gRPC
boundary and the native Elixir gRPC client. The platform team owns common
types, options, compatibility policy, and generation. The application team
owning a domain owns that domain's messages and service methods; changes still
require protocol-owner review because all packages are public internal wire
contracts.

Packages are versioned independently in their protobuf package name. Breaking
changes create a new package version. Removed field names and numbers remain
reserved. The repository checks `buf lint`, formatting, compatibility against
the configured Git baseline, descriptor policy, and deterministic regeneration.

Generated Rust and Elixir files are checked in so normal Cargo and Mix builds
do not download generators. They are never hand edited. Run
`scripts/generate-protobuf.sh`, or `scripts/check-generated.sh` to regenerate
in a temporary worktree and verify that the committed output is clean.

## Reviewed toolchain

- Rust and Cargo: 1.88.0, the MSRV declared and CI-verified by connect-rust.
- Connect runtime: `connectrpc` 0.8.1.
- Connect generator: `connectrpc-codegen` 0.8.0 (the generator published by the
  connect-rust v0.8.1 source release).
- Buffa message and packaging generators: 0.8.1.
- Buf CLI: 1.72.0.
- Elixir protobuf generator/runtime: 0.17.0.
- Elixir gRPC generator/runtime: 1.0.2.

The Connect release is pre-1.0 and therefore remains exact-pinned. Upgrade it
only with regenerated outputs, compatibility checks, and protocol interop
tests.

The first revision that adds `proto/` is the reviewed v1 compatibility
baseline, so there is no older schema against which Buf can compare it. The
breaking-change check reports that one bootstrap case explicitly when
`origin/main` exists without a `proto/` tree. After this revision reaches the
main branch, every subsequent check compares against `origin/main:proto` (or
the explicit `HEPHAESTUS_BUF_BREAKING_AGAINST` baseline supplied by CI).

## Identity and request metadata

Ordinary application requests never contain an actor selector. Phoenix sends
`authorization: Bearer <JWT>` with HS256 claims: issuer
`hephaestus-web-mediator`, subject equal to the internal user UUID, audience
equal to the exact method path (`/hephaestus.<domain>.v1.<Service>/<Method>`),
UUID `jti`, and `iat`/`nbf`/`exp` no more than 30 seconds apart. Servers permit
at most five seconds of clock skew. The signing key is the SHA-256 digest of
the domain separator `hephaestus-rpc-mediator-v1\0` followed by the configured
high-entropy internal token. The original OIDC issuer and subject are used only
by `IdentityService.ResolveIdentity` and are not propagated to domain RPCs.

`ResolveIdentity` uses a distinct bootstrap assertion because the internal user
UUID does not exist yet. It uses the same signing key, issuer, method audience,
TTL, skew, and `jti` rules, but `sub` is the service principal
`hephaestus-web-mediator` and the required `actor_kind` claim is
`verified_oidc_bootstrap`. Signed claims `oidc_iss`, `oidc_sub`, `name`,
`email`, and `email_verified` must exactly match the typed request; the server
rejects any mismatch. After resolution, every ordinary assertion uses the
returned internal UUID as `sub` and omits all OIDC claims.

Mutations additionally carry `hephaestus.common.v1.RequestContext`. Its request
and idempotency keys, rather than JWT `jti`, define safe retry identity.

## Type policy

IDs are opaque UUID wrappers. Lists use the shared page request/response and
declare stable ordering. Cursors are opaque, scope-bound resume positions.
Known data uses typed messages and `oneof` values; `Struct`, arbitrary JSON,
maps, and actor fields are forbidden. Field masks are intentionally absent
until a real partial-update operation exists. Secret plaintext appears only in
`SecretValue.value` and is marked with the custom sensitive option.

## Product events and watches

`hephaestus.event.v1.ProductEvent` is the only client-facing event envelope.
Its cursor is monotonic only within its explicit identity, organization,
project, repository, run, or agent-instance scope. Event IDs are globally
stable for deduplication; aggregate versions detect gaps and impossible
reordering. The payload `oneof` is kept in exact parity with
`proto/event-reducer-coverage.toml`.

Identity profile invalidations expose only typed change/lifecycle state. OIDC
issuer, external subject, display name, and email are never event payloads.

There is deliberately no global watch. `WatchIdentity` derives its identity
scope from authenticated metadata and accepts no identity selector. The other
watch methods take one authorized resource scope. A new watch emits a
transactional `ScopeSnapshotBarrier` first; clients buffer later events, load
ordinary RPC snapshots, and then reduce the buffer. Resume, retention gaps,
access revocation, event-count budgets, and byte budgets are explicit protocol
states.

Every mutation response includes `hephaestus.common.v1.MutationReceipt` so a
client can read its own write against the committed cursor and aggregate
version. Its scope is operation-defined: identity bootstrap uses identity;
build requests use repository; run control uses the target run; agent import
uses project; later instance commands use agent instance; secret lifecycle
uses the secret owner; and grant/import commands use their target.

Product payloads contain only safe typed projections and related opaque IDs.
Their domain detail is limited to typed change/lifecycle state; ref names and
commit values are reloaded through ordinary snapshot RPCs. They never contain
secret plaintext or ciphertext, credentials, tokens, command parameters or
environments, arbitrary JSON/bytes, high-volume logs or metrics, or
unrestricted diagnostics.
