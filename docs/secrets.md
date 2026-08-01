# Secret delegation and runtime delivery

Hephaestus has no generic `secret.read` and no endpoint that returns a stored
plaintext value. Secret authority is split across metadata inspection, value
submission, rotation, grant management, import acceptance, binding, raw guest
receipt, brokered use, revocation, and purge.

```text
organization/project-owned secret
→ exact target grant
→ target-accepted opaque import
→ immutable revision binding to a declared slot
→ exact version resolution at dispatch
→ ephemeral raw file or host-only semantic broker operation
```

## Ownership, storage, and lifecycle

A secret has exactly one organization or project owner. Repository scopes
import organization or containing-project secrets; repositories never own
values. Cross-organization grants, imports, attachments, bindings, and leases
fail closed.

Each submitted value creates an immutable encrypted `secret_version`.
`secret-store` uses AES-256-GCM data encryption plus AES-256-GCM key wrapping,
unique nonces, a versioned host key reference, and authenticated associated
data binding the owner, secret, version, algorithm, and immutable metadata.
PostgreSQL stores ciphertext and wrapping metadata, never plaintext.

Host key material is provisioned outside PostgreSQL as a bounded map from
versioned key reference to 256-bit wrapping key. Startup rejects an empty,
malformed, duplicate, or unavailable active key set and never substitutes a
plaintext store. Rotating the host wrapping key changes the active reference
for later versions; prior keys must remain available while retained versions
or backups still reference them.

A database backup contains ciphertext, nonces, wrapped data keys, authenticated
metadata, and key references, but it is not independently decryptable. The
corresponding external key set must be backed up through the operator's secret
custody system. Restore with the exact key reference and bytes recovers the
value only through the authorized ephemeral resolver. A missing reference,
wrong key, modified nonce/ciphertext, or transplanted metadata fails
authentication closed. Operators should verify a restore before retiring an
old wrapping key and purge that key only after every retained version and
backup using it has expired.

Rotation creates and atomically activates a new version. Existing runs remain
pinned to their resolved version unless it is explicitly revoked. Revocation
blocks new resolution immediately, ends broker authority live, and requests
guest cancellation for raw leases. It does not falsely claim that bytes
already observed by a guest were erased. Purge removes encrypted material only
after leases and retention checks permit it while keeping tombstone/audit
provenance.

## Grants, imports, and bindings

Organization membership, repository visibility, and public access imply no
secret authority. A source manager grants one exact project or repository a
bounded set of modes, phases, destinations, and optional expiry. A manager of
that target independently accepts the grant under a local alias. The import is
an opaque live reference—not copied ciphertext—and cannot be re-exported.

The compound grant-and-import command is one transaction and succeeds only
when the same actor independently has both source `manage_grants` and target
`accept` authority.

A release declares symbolic slots such as `model` or `deploy`, accepted modes,
phases, purpose, and broker constraints. Declarations grant no tenant
authority. Binding a slot validates the exact import, instance, revision,
selected attachments, target scope, release declaration, delivery mode,
phase, and platform policy in one transaction. A new immutable revision and
binding are created; only opaque IDs and normalized policy are stored.

## Dispatch and runtime credentials

Dispatch reauthorizes the run/update, instance, revision, attachment, grant,
import, binding, secret/version lifecycle, phase, mode, and destination. It
pins one immutable version in `run_secret_provenance`, creates exact leases,
and returns a short-lived opaque runtime credential. PostgreSQL stores only its
hash. NATS and queued commands contain neither values nor reusable runtime
credentials.

Runtime credentials are bound to the run, instance, revision, attachment,
phase, binding set, and expiry. Cross-run, cross-slot, cross-attachment,
stale, expired, or revoked use is denied.

After materialization, the orchestrator rechecks the exact session, every
lease, and the live binding/import/grant/secret/version chain immediately
before VM provisioning. A denial removes the ephemeral mount and runtime
without provisioning a guest. The daemon also supervises revoked raw
sessions: revocation during provisioning or execution becomes a durable run
cancellation, after which normal destroy-before-mount-cleanup ordering applies.
Broker-only sessions are denied live without requiring guest cancellation.

Secret creation, rotation, grants, import acceptance, binding, runtime
authority issuance, revocation reconciliation, and purge update the canonical
application-event journal in the same transaction as the authoritative state
change. Typed, authorization-scoped product-event watches deliver those
changes to the UI. The superseded `hephaestus.secret.*.v1` JSON outbox records
and the undocumented `HEPHAESTUS_SECRET_EVENTS` stream are retired; secret
values and reusable credentials are never placed in a transport payload.

## Raw delivery

Raw authority is explicit and more sensitive. After VM resources are ready,
the host creates one memory-backed per-run secret tree and stable slot-derived
files, then mounts it read-only at `/run/hephaestus/secrets`. Values never
enter environment variables, arguments, ordinary configuration, source,
release, result, state, logs, metrics, NATS, or PostgreSQL metadata.

The guest can read and copy a raw value; this mode is not described as
non-disclosing. The guest is destroyed before the host removes the ephemeral
tree and releases the lease. Restart reconciliation removes only mounts that
cannot belong to a live guest.

## Brokered delivery

Brokered authority gives the guest an opaque capability, not the credential.
A dedicated libkrun vsock mapping terminates at a private host Unix socket.
Broker-only guests receive no general IP network.

Each broker call authenticates the runtime-token hash, checks its exact
context, reauthorizes live Mélange permission, enforces slot/operation/
destination policy, and applies the decrypted credential only inside a
host-side semantic adapter.

The initial completion adapter is deliberately narrow:

- one trusted loopback socket and exact validated logical DNS destination;
- one fixed `POST /v1/complete` operation and path;
- host-applied bounded Bearer value;
- no redirects, guest-selected addresses/paths, raw IPs, IPv6 literals,
  local/internal metadata names, tunneling, or response-header forwarding;
- bounded request/response bodies, timeout, concurrency limit, strict HTTP
  status handling, and a `deny_unknown_fields` `{result}` response;
- rejection of credential echoes, control characters, malformed responses,
  and provider debug bodies.

## Audit and UI safety

Secret audit records contain opaque IDs, requester/mediator/runtime,
permission, target, mode, authorization-model version, decision, request or
command identity, and outcome. They never contain values.

The project settings UI lists authorization-filtered metadata and explicitly
states that values are unavailable by design. Any future write-only form must
not prepopulate or retain a value in LiveView state, reconnect payloads,
flashes, URLs, browser logs, screenshots, or telemetry.

## Operator runbook

The local daemon loads a versioned wrapping-key ring from
`HEPHAESTUS_SECRET_KEY_DIRECTORY`. The directory must be absolute,
service-owned, and mode `0700`. Each entry is a non-symlink regular file whose
filename is the key reference, whose mode is exactly `0400`, and whose contents
are exactly 32 raw random bytes. `HEPHAESTUS_SECRET_KEY_REFERENCE` names the
active entry. Startup fails closed for unsafe modes, wrong ownership,
symlinks, malformed lengths, invalid references, an empty ring, or a missing
active reference. Key bytes no longer belong in environment variables.

To rotate wrapping keys:

1. Generate 32 random bytes through the host secret-custody system into a new
   temporary file outside the key directory.
2. Set ownership and mode `0400`, move it atomically into the `0700` key
   directory under a new versioned reference, and retain every older file.
3. Change only `HEPHAESTUS_SECRET_KEY_REFERENCE` and restart the daemon.
4. Create and resolve a test version, verify backup restore with the complete
   ring, and only then use the new active key for normal writes.
5. Retire an old file only after no retained database version or backup names
   its reference.

An unavailable old key is a custody incident, not a cue to substitute another
key. Stop rotations and purges, restore the exact referenced key from the
external key backup, verify its ownership/mode and a controlled decrypt, then
restart. A wrong key continues to fail authenticated decryption.

Operational inspection must remain metadata-only. Use the authorization-
filtered secret, grant, import, binding, runtime-session, lease, and
`secret_audit_events` views/tables; never query ciphertext columns into logs or
support bundles. Emergency revocation uses the normal authorized revoke
command so grant/import/binding/session invalidation and raw-run cancellation
remain durable and auditable. The supervised daemon then cancels affected raw
guests, denies broker calls live, and removes ephemeral mounts only after guest
destruction. On restart, normal reconcilers resume unpublished outbox records,
revoked-session cancellation, runtime cleanup, and safe orphan removal without
minting replacement versions or credentials.

The current composition root deliberately installs `DenyingBrokerAdapter`.
Production KMS/vault providers and additional semantic upstream adapters are
separate follow-up work; operators must not treat the local key provider or
fake loopback adapter as a production credential boundary.

Migration `0009_operational_observability.sql` adds metadata-only secret
version and aggregate operational views. The application role still has no
privilege on `secret_versions`; only the trusted worker may access encrypted
records. `hephaestus-operator inspect-secret` reads the safe view, while
`metrics` reports opaque aggregate counts for rotations, live leases, denied
resolutions, broker use, raw-delivery runs, and active raw mounts.

The browser E2E uses a unique plaintext sentinel and fails if it appears in
rendered HTML, browser console output, screenshots, Phoenix/daemon/OIDC logs,
the PostgreSQL dump, JetStream storage, repositories, workspaces, result
artifacts, runtime files, or the post-run ephemeral secret root. Phoenix
recursively filters secret/token/password LiveView parameters before debug
logging. Sentinel scanning detects exact known values; a malicious guest can
arbitrarily transform a raw value, so it cannot establish non-disclosure after
raw receipt. Raw mode must be treated as deliberate plaintext delegation to
the guest.
