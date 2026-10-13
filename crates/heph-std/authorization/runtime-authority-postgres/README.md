# Purpose

`runtime-authority-postgres` persists immutable authorization snapshots and
hash-only runtime sessions. It gives trusted workers and gateway admission code
the database side of exact runtime authority issuance.

# Responsibilities

`PgRuntimeSessionRepository::resolve_snapshot` verifies that a run is in the
correct provisioning state, selects its exact revision bindings, and builds the
normalized snapshot. `live_authorized` rechecks every snapshotted operation
against current authorization immediately before provisioning. The gateway
issuer performs the equivalent revision and handler-contract checks for guest
or host-mediated gateway sessions, while persisting no plaintext bearer.

The separate `LiveRunSnapshotResolver` implementation is disabled by default.
Explicit `with_canonical_live_snapshots` configuration requires a worker role,
an owned execution profile, exact planned VM scope, and immutable producer
provenance. Normal runs use their exact authenticated run request; update hooks
use their provision-consumed VM admission actor and request, never the mutable
update actor. Historical, mailbox, and unadmitted invocation provenance fails
closed. The existing Provisioning-only issuance query remains unchanged.

Pre-issuance checks accept LeasingVolume or Provisioning and reject acquisition
closure, cancellation, and permanent removal. Issued checks require the exact
persisted snapshot/session pins, a fresh pending or active unexpired session,
and the unchanged generic ceiling. They accept Provisioning, Starting, Running,
Succeeded, Failed, or CleaningUp for source checks until physical confirmation;
intentional acquisition closure alone does not deny this read-only check.
Cancellation, permanent removal, changed released pins, or withdrawn source or
generic operation authority still denies it. No issued check permits new runtime
effects in a terminal phase.

Typed mount grants and caller execution rights remain the canonical launch and
volume-preflight responsibility; secret checks remain separate. This resolver
does not activate canonical application dispatch or issue running/draining
credentials. Native live-phase validation is a separate gate from the existing
profile and issuance tests.

`recorded_live_session_ceiling` is a separate, explicitly configured worker-only
read. It loads stable session facts and the actual stored snapshot hash using
the same codec as issued validation. Only complete absence of both durable
records returns `None`; orphaned or inconsistent records and revoked or elapsed
sessions fail closed. The app manager uses this ceiling for reauthorization
without creating a session, rotating credentials or writing a handoff. This
lookup alone grants no source, caller or mount authority.

`recorded_live_source_context` reuses the same owned phase/provenance query to
return non-serializable original actor/request facts and exact run pins. It is
worker-only and explicitly configured. This projection is not authentication or
permission: the app must validate the live snapshot and audit current caller and
source rights separately. The shared query rejects mailbox and unknown history,
and reads update principals only from provision-consumed immutable admissions.

# When

Create `PgRuntimeSessionRepository` over a worker-role pool while preparing an
agent run, then pass its snapshot and identity to the core runtime issuer. Use
`PgGatewayRuntimeAuthorityIssuer` for accepted gateway invocations and choose
the guest or host-mediated issuance method according to the handler contract.

The trusted `volume_mount_authorized` helper checks an exact active normal run,
revision, declared slot, resource, access mode and published contract hash.
Bindings and generic `agent_attach` alone are insufficient. Both creator source
grant and attachment authority must remain live; explicit revocation is permanent.
Callers must check before acquiring a lease, before starting the VM, and during
heartbeat. Runtime wiring and plural attachments remain deferred. Denial requires
bounded VM stop/fencing before lease release; a SQL check does not unmount a VM.

# Qualified Invocation authority

`with_qualified_invocation_configuration` is a separate default-off option. It
requires the private 115/117/119-A admission and lease schema plus private
119-B1 authority functions; public 107 does not supply those functions. The
checked configuration is server metadata, not provider ownership or permission.
The global named dispatch helper stays false. Normal and Update query bodies,
TTL policy and credential lifecycle remain unchanged.

New sessions and pending redelivery require Provisioning, exact saved invoker
Execute and SourceUse rights, current issuer delegations, the complete held lease
set, and the original configured provider/revision pins. Issuance copies the
whole immutable capability ceiling and hash-only verifier in one transaction.
Required or optional Git authority is unsupported. Monitor reads may use an
existing acknowledged ceiling in Starting/Running only after exact attachment
markers; they never issue, renew or enlarge it. Revocation and expiry retain the
same identity, ceiling, generation and verifier after execution rights are lost.

This qualified path supports the existing Core 64-bit length framing only.
Issue and expiry must already be representable as PostgreSQL microseconds before
constructing the Core identity; the writer compares their original nanosecond
decimal claims without rounding. Expiry is at most 3600 seconds after issue and
at most every held lease expiry, including the positive zero-volume admission.
Ordinary session policy and historical hashes are untouched. Secret, App/VM
composition and physical guest execution remain separate unsupported work.

# Qualified first acquisition

`verify_qualified_invocation_pre_acquisition` requires the separate private119
pre-acquisition function and explicit qualified configuration. Trusted server
composition supplies its authorization model constant. The worker transaction
uses a repeatable read, read-only snapshot and compares the supplied Run's pins
and state version with the actual consumed115/117 LeasingVolume record. It checks
current actor, source, capability issuer and whole mount eligibility, saved empty
secrets, exact configuration/model and absence of any lease, ceiling, cleanup
or Run-owned secret history, including released or revoked records. Zero slots
use the same proof. Full B2 revision contradiction checks remain mandatory at
actual Provisioning before provider IO.

Success is first-acquisition eligibility only. It returns no authority data and
cannot issue credentials, authorize provider IO or replace B1/B2 after acquisition.
Other phases and default unconfigured repositories deny. Existing B1 held-set and
B2 Provisioning-only contracts stay unchanged. Core deferral and App routing are
separate pending composition work; this method does not enable that pipeline.
