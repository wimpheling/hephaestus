# Manage, delegate, and deliver secrets without disclosure

Owner: unassigned

## Outcome

Add organization- and project-owned secrets that can be shared deliberately
with projects and repositories, bound to exact agent-instance revisions, and
used by authorized runs without requiring the person configuring the agent to
see the secret value.

The intended flow is:

```text
organization or project owns a versioned secret
→ secret manager grants an exact project or repository permission to import it
→ target manager accepts an opaque import under a local alias
→ authorized user binds the import to a declared agent capability slot
→ run dispatch reauthorizes and pins an exact secret version
→ guest receives either a raw ephemeral file or an opaque broker capability
→ use, denial, rotation, and revocation are audited without recording the value
```

“Access to a secret” is not one permission. Reading metadata, submitting a new
value, rotating it, granting its use, accepting an import, binding it to an
agent, receiving the raw value, and exercising it through a broker are distinct
operations. In particular, a user may be allowed to import or bind a secret
without any API capable of revealing its value.

This task supplies the secret-management and runtime-delivery work referenced
by
[`reusable-agent-releases-and-instances.md`](reusable-agent-releases-and-instances.md).

## Locked decisions

| Area | Decision |
| --- | --- |
| Secret ownership | A secret is owned by exactly one organization or one project. Repositories do not own secret values; they import organization or containing-project secrets. |
| Tenant boundary | Secrets and imports never cross organization boundaries. |
| Imports | An import is an opaque reference and local alias, not a copy of ciphertext or plaintext. Rotation and revocation continue to originate at the owning secret. |
| Explicit sharing | Organization membership, project membership, repository visibility, and public repository access grant no secret authority implicitly. Every project or repository import requires an explicit source grant and target acceptance. |
| Non-transitivity | Imports cannot be re-exported. A repository importing an organization secret needs a grant from that organization secret; possession of a project import is not enough to launder it onward. |
| No plaintext reveal | The initial product has no endpoint or UI action for retrieving an existing plaintext value. A manager can submit or replace a value but cannot read it back afterward. |
| Delegation without disclosure | A user may accept an import and bind it to an agent when authorized by both the source grant and target scope, without plaintext access. |
| Two-sided authorization | A source secret manager creates an active grant for an exact target, and a target secret manager accepts it. The accepting user does not need source-management authority: the grant carries that decision. One actor may perform both steps only when independently authorized on both sides. |
| Agent authority | An agent instance inherits no project or repository secret authority. An immutable binding grants only an exact declared slot, target scope, execution phase, delivery mode, and optional destination constraint. |
| Repository scope | A repository import is usable only for runs whose exact attachment and target repository match that import. A project import may be bound across explicitly selected attachments in that project; it is never silently available to every agent. |
| Release declarations | A release may declare symbolic secret or credential slots, required delivery modes, purpose, allowed phases, and broker constraints. A declaration grants no authority and names no tenant secret. |
| Revision bindings | An agent-instance revision binds declared slots to `SecretImportId` values and contains no plaintext or ciphertext. Changing a binding creates a new immutable revision. |
| Exact use | Dispatch resolves the binding to one immutable `SecretVersionId`, records it in protected run provenance, and mints any runtime lease only after live authorization. Queued commands and NATS messages never contain values or reusable credentials. |
| Delivery modes | Raw delivery and brokered use are different authorities. `receive_raw` permits the guest to read the value from an ephemeral file. `use_brokered` gives the guest only an opaque capability while the host applies the credential to an allowed operation. |
| Raw-delivery honesty | A guest receiving a raw value can copy or exfiltrate it. UI, policy, and audit must not describe raw delivery as non-disclosing merely because the configuring human could not see the value. |
| No environment delivery | Raw values are never placed in process environment variables, command arguments, configuration JSON, durable workspaces, state volumes, result artifacts, logs, metrics, traces, or crash diagnostics. |
| Encrypted storage | Plaintext is never stored in PostgreSQL, NATS, release storage, or Git. Each immutable secret version is authenticated-encrypted under a versioned host-side key reference using a reviewed cryptographic implementation. |
| Rotation | Rotation creates a new immutable version and atomically changes the active version. New dispatches use the new version; already started runs remain pinned to the version in their provenance unless that version is explicitly revoked. |
| Revocation | Revocation blocks new resolution immediately. Broker calls reauthorize live and stop immediately. A raw value already delivered to a guest cannot be clawed back; revocation requests cancellation and destruction of affected guests without claiming erasure from guest memory. |
| Deletion | Deletion is a tombstone plus cryptographic-material purge after active leases permit it. Metadata and audit provenance remain resolvable without retaining plaintext or usable ciphertext forever. |
| Audit | Creation, value replacement, rotation, grant, import, binding, resolution, use, denial, revocation, and purge are audited with IDs and policy versions, never secret values. |
| Worker trust | Trusted workers may resolve an already-authorized durable command mechanically, but agent-facing broker APIs use a non-`BYPASSRLS` role and perform live Mélange permission checks. |

## Permission vocabulary

The authorization model must avoid a generic `secret.read` permission:

| Permission | Meaning |
| --- | --- |
| `secret.inspect_metadata` | See the secret's name, owner, status, allowed delivery modes, and rotation metadata, but no value. |
| `secret.write_value` | Submit an initial or replacement value. It does not retrieve the prior value. |
| `secret.rotate` | Create and activate a new immutable version. |
| `secret.manage_grants` | Offer bounded import authority to an exact project or repository. |
| `secret.revoke` | Disable the secret, a version, or a grant and start affected-runtime reconciliation. |
| `secret.purge` | Irreversibly destroy retained encrypted material after safety checks. |
| `secret_import.accept` | Accept a source grant and choose a local alias in a managed target. |
| `secret_import.bind_brokered` | Bind the import to an agent slot using a permitted broker adapter. |
| `secret_import.bind_raw` | Bind the import for raw guest delivery; this is distinct and more sensitive. |
| `secret.use_brokered` | Let an exact authorized runtime exercise a broker operation without receiving plaintext. |
| `secret.receive_raw` | Let an exact authorized runtime receive plaintext in its ephemeral secret mount. |

There is intentionally no initial `secret.reveal` permission.

## Sharing semantics

### Organization secret to project

An organization secret manager creates a grant naming one exact project and
the allowed delivery modes, agent phases, destinations, and expiration if any.
A project secret manager accepts it under a project-local alias. Project
maintainers may then bind that import only when the import policy permits it
and only to agent instances and attachments they can configure.

### Organization secret to repository

An organization secret manager may grant an exact repository directly. A
repository secret manager accepts the import. The import cannot be used by a
run against another repository, even when the same agent instance has several
attachments.

### Project secret to repository

A project secret manager may grant a project-owned secret to a repository in
that same project. The repository accepts an opaque import under its own alias.
No other project or organization may receive it.

### Agent binding

A release declaration such as `model`, `github`, or `deploy` is a symbolic
slot. An instance revision maps that slot to an eligible project or repository
import, a delivery mode, allowed normal/update phases, selected attachments,
and broker constraints. Binding authorization requires all of:

```text
user may configure the exact agent instance
AND user may bind the selected secret import in the requested delivery mode
AND every selected attachment is inside the import's target scope
AND the release declaration accepts that delivery mode and phase
AND current platform policy permits the resulting capability
```

The user does not need and cannot obtain the stored plaintext through this
workflow.

## Non-goals

This task does not add cross-organization sharing, repository-owned secret
values, plaintext reveal/download, environment-variable delivery, secrets
committed to source configuration, guest Git credentials, automatic grant
inheritance, arbitrary import re-export, or secrets for untrusted builds.

A general transparent TLS-intercepting credential proxy, arbitrary protocol
credential injection, external-vault synchronization, hardware-backed KMS
integration, and long-lived service credential renewal may be split into
focused follow-up tasks. The initial broker must nevertheless prove the
non-disclosing model end to end with at least one application-level adapter and
a fake upstream service.

## Dependencies and affected boundaries

This work depends on project-owned agent instances and immutable revisions,
provider-neutral runtime credentials, exact run provenance, constrained guest
networking, PostgreSQL/Mélange authorization, RLS, transactional outboxes, and
the Phoenix project UI. It affects agent configuration, run and update
dispatch, VM mounts and vsock protocols, storage encryption, authorization,
audit, retention, and organization/project/repository settings.

## Implementation checklist

- [ ] **1. Define provider-neutral secret contracts**
  - [ ] **Add stable identifiers and bounded values**
    - [ ] Add `SecretId`, `SecretVersionId`, `SecretGrantId`,
      `SecretImportId`, `AgentSecretBindingId`, and `SecretLeaseId`.
    - [ ] Add validated secret names, local aliases, declared slot keys,
      delivery modes, phases, destination constraints, statuses, and bounded
      opaque values.
    - [ ] Use redacted secret-value wrappers whose `Debug`, `Display`,
      serialization errors, and tracing fields cannot expose contents.
  - [ ] **Define lifecycle models**
    - [ ] Define organization/project ownership, immutable versions, active
      version selection, grants, imports, revision bindings, runtime leases,
      revocation, and purge state machines.
    - [ ] Define deterministic idempotency keys for create, rotate, grant,
      accept, bind, resolve, revoke, and purge commands.
    - [ ] Define structured non-sensitive diagnostics for missing, revoked,
      expired, out-of-scope, wrong-mode, and unauthorized bindings.
  - [ ] **Add domain tests**
    - [ ] Test parsing, bounds, serialization, redaction, and malformed-input
      rejection.
    - [ ] Test valid and invalid lifecycle transitions, tenant boundaries,
      scope matching, and deterministic identities.
    - [ ] Add tests that intentionally format every error and domain value and
      prove a sentinel secret never appears.

- [ ] **2. Store immutable encrypted secret versions**
  - [ ] **Define the storage boundary**
    - [ ] Add a provider-neutral encrypted secret store that accepts plaintext
      only at create/rotate and returns it only to an authorized ephemeral
      resolver.
    - [ ] Use authenticated encryption from a reviewed library, unique nonces,
      per-version data keys or equivalent isolation, and a versioned
      key-encryption-key reference outside PostgreSQL.
    - [ ] Bind owner, secret, version, algorithm, and immutable metadata as
      authenticated associated data so ciphertext cannot be transplanted.
    - [ ] Define host key loading, startup validation, rotation, backup,
      restoration, and unavailable-key behavior without silently falling back
      to plaintext.
  - [ ] **Add PostgreSQL records**
    - [ ] Add secrets with exactly one organization or project owner, scoped
      unique names, status, active version, policy metadata, creator, and
      timestamps.
    - [ ] Add immutable secret versions with ciphertext storage reference,
      algorithm, nonce/key metadata, content length, creator, and timestamps.
    - [ ] Add grants, imports, immutable agent bindings, runtime leases, and
      tombstones with complete foreign keys and compare-and-swap constraints.
    - [ ] Prevent cross-organization ownership, grants, imports, attachments,
      and agent bindings with database constraints where representable.
  - [ ] **Implement rotation and purge**
    - [ ] Create and activate a new version transactionally without rewriting
      existing versions or revision bindings.
    - [ ] Revoke a secret, version, or grant immediately for new resolution and
      enqueue affected-runtime reconciliation.
    - [ ] Purge encrypted material only after active raw/broker leases and
      retention rules permit it while preserving tombstone provenance.
  - [ ] **Add storage tests**
    - [ ] Test ciphertext and associated-data tampering, wrong keys, nonce
      uniqueness, unavailable keys, key rotation, backup/restore, and purge.
    - [ ] Inspect PostgreSQL, NATS fixtures, logs, and filesystem artifacts to
      prove plaintext is absent.
    - [ ] Test concurrent rotations and stale active-version CAS rejection.

- [ ] **3. Implement grants, imports, and non-disclosing authorization**
  - [ ] **Extend the canonical OpenFGA model**
    - [ ] Add secret, secret grant, secret import, agent secret binding, and
      runtime lease object types with the permission vocabulary above.
    - [ ] Add explicit organization and project secret-manager relations rather
      than deriving secret authority from ordinary membership or repository
      read access.
    - [ ] Let project/repository managers accept imports and bind them only
      when a live source grant authorizes that exact target, mode, and scope.
    - [ ] Give agent runtimes only the exact brokered-use or raw-receive
      relation produced by an active binding and lease.
    - [ ] Ensure public repositories and unrelated project members receive no
      secret metadata or authority.
  - [ ] **Update Mélange, RLS, and actor context**
    - [ ] Generate and commit specialized permission SQL from the canonical
      model and update drift and compatibility fixtures.
    - [ ] Derive `melange_tuples` from authoritative ownership, grants, imports,
      bindings, instances, attachments, runs, and leases.
    - [ ] Apply forced RLS to all secret metadata and audit tables while
      isolating ciphertext access behind the narrow resolver boundary.
    - [ ] Support user and runtime subjects without granting agent-facing
      requests the trusted worker role.
  - [ ] **Authorize compound operations**
    - [ ] Require an active source-issued grant for the exact target plus the
      accepting actor's target-management permission; do not require that actor
      to manage the source secret.
    - [ ] Permit an atomic grant-and-import convenience command only when the
      same actor independently passes both source- and target-side checks.
    - [ ] Require agent configuration, import binding, attachment scope,
      release-slot, delivery-mode, phase, and platform-policy authorization in
      one transaction when creating a binding.
    - [ ] Reauthorize grant, import, binding, instance, attachment, run/session,
      delivery mode, and secret/version status at dispatch and on every broker
      call.
    - [ ] Prevent a user allowed to bind or use a secret from rotating,
      granting, purging, or retrieving it.
  - [ ] **Add authorization tests**
    - [ ] Cover organization owners, explicitly assigned secret managers,
      project maintainers, repository managers, ordinary members, outsiders,
      agent instances, and exact run subjects.
    - [ ] Test organization-to-project, organization-to-repository, and
      project-to-repository imports, including every cross-tenant denial.
    - [ ] Test that `bind_brokered` never implies `bind_raw`, and neither
      implies value management or grant management.
    - [ ] Test unknown-object denial, RLS list filtering, normal-role
      non-bypass, revocation, and Mélange/OpenFGA parity.

- [ ] **4. Extend release declarations and instance bindings**
  - [ ] **Add symbolic secret slots**
    - [ ] Extend the versioned release configuration with bounded stable slot
      keys, human-readable purpose, required/optional state, allowed phases,
      accepted delivery modes, and broker/destination constraints.
    - [ ] Reject tenant secret IDs, aliases, and plaintext values in source
      configuration.
    - [ ] Bind the normalized declaration and its hash into the immutable
      release agent.
  - [ ] **Resolve instance revisions**
    - [ ] Map each configured slot to an eligible `SecretImportId`, delivery
      mode, selected attachments, execution phases, and broker constraints.
    - [ ] Validate required slots and persist only opaque IDs and normalized
      effective policy in the immutable instance revision.
    - [ ] Mark a revision visibly unrunnable when a required import is missing,
      revoked, expired, out of scope, or unsupported by current platform
      delivery capabilities.
    - [ ] Make binding changes create new revisions without rewriting
      historical runs or bindings.
  - [ ] **Add binding tests**
    - [ ] Test project imports across several repository attachments and strict
      repository-import isolation on a multi-attachment instance.
    - [ ] Test required, optional, normal-run-only, and update-hook-only slots.
    - [ ] Test grant narrowing, import revocation, release changes, and platform
      policy changes produce stable diagnostics rather than plaintext errors.

- [ ] **5. Resolve exact versions and issue runtime authority**
  - [ ] **Resolve at dispatch**
    - [ ] Resolve every binding to the current active immutable secret version
      only after all live authorization and lifecycle checks pass.
    - [ ] Persist protected run provenance containing secret, version, import,
      binding, grant, authorization-model, and delivery-policy IDs without
      values.
    - [ ] Mint short-lived opaque runtime credentials and secret leases bound
      to the exact instance, revision, run/update, attachment, versions,
      binding set, phases, expiry, and capability ceiling.
    - [ ] Store only runtime-token hashes and never place a previously minted
      token in queued work or NATS.
  - [ ] **Handle rotation and revocation**
    - [ ] Make rotation affect only later dispatches unless an old version is
      explicitly revoked.
    - [ ] Deny new leases immediately after secret, version, grant, import, or
      binding revocation.
    - [ ] Reconcile active broker leases immediately and request cancellation
      and destruction for guests holding revoked raw values.
    - [ ] Record honestly when raw material may already have been observed and
      cannot be withdrawn.
  - [ ] **Add dispatch tests**
    - [ ] Test exact version pinning across rotation, retries, NATS redelivery,
      update hooks, and concurrent dispatch.
    - [ ] Test every revocation race before resolution, after lease creation,
      during guest provisioning, and after raw or brokered use begins.
    - [ ] Test that expired credentials, stale leases, and mismatched
      run/revision/attachment contexts fail closed.

- [ ] **6. Deliver raw secrets through an ephemeral mount**
  - [ ] **Construct the mount**
    - [ ] Materialize raw values only after VM resources are ready into a
      per-run host-controlled ephemeral secret filesystem.
    - [ ] Mount it read-only at `/run/hephaestus/secrets` with one stable
      slot-derived filename, restrictive mode and ownership, and no
      user-controlled host paths.
    - [ ] Exclude the mount from source, release, work, result, state, snapshot,
      debug-bundle, and backup importers.
    - [ ] Pass non-secret slot/path metadata separately from values.
  - [ ] **Finalize safely**
    - [ ] Destroy the guest before destroying the ephemeral secret filesystem
      and release lease state only after provider cleanup is confirmed.
    - [ ] Reconcile crashes and orphaned secret mounts without exposing paths or
      retaining reusable plaintext.
    - [ ] Bound secret sizes/counts and reject symlinks, special files, aliases
      that collide after normalization, and unsupported guest providers.
  - [ ] **Add real-guest tests**
    - [ ] Verify exact contents, ownership, modes, read-only behavior, absence
      from `/proc` environment and arguments, and removal after destruction.
    - [ ] Attempt to copy secrets into results, logs, crashes, and state and
      verify platform-controlled collectors redact or reject known values while
      documenting that malicious arbitrary transformations cannot be detected
      reliably.
    - [ ] Force termination and provider failure at every materialization and
      cleanup boundary and verify no reusable mount remains.

- [ ] **7. Implement non-disclosing brokered use**
  - [ ] **Define the broker protocol**
    - [ ] Let a guest present its opaque runtime credential, symbolic slot,
      requested operation, destination, and bounded request without presenting
      a secret value.
    - [ ] Authenticate the exact runtime, enforce its lease/capability ceiling,
      perform live Mélange checks, validate destination and operation policy,
      and apply the credential only outside the guest.
    - [ ] Return bounded sanitized responses and never echo upstream
      authorization material, provider debug bodies, or secret-bearing headers.
    - [ ] Prevent direct guest egress from bypassing the broker for a
      broker-only binding.
  - [ ] **Add an initial application-level adapter**
    - [ ] Implement one narrow adapter against a fake upstream service that
      proves host-side credential application, destination binding, rotation,
      revocation, rate limiting, and audit.
    - [ ] Keep adapter-specific operations semantic and allowlisted rather than
      exposing a generic credential-fetch endpoint.
    - [ ] Define provider-neutral failure and retry semantics that never include
      values in diagnostics.
  - [ ] **Add broker security tests**
    - [ ] Test token theft across runs, instances, revisions, slots,
      attachments, destinations, and expired/revoked leases.
    - [ ] Test alternate DNS, raw IP, redirects, IPv6, metadata endpoints,
      tunneling, oversized bodies, malicious headers, and response leakage.
    - [ ] Prove through a fake-upstream sentinel that the guest never receives
      the brokered credential while the authorized operation succeeds.

- [ ] **8. Add durable commands, audit, and observability**
  - [ ] **Publish safe commands and events**
    - [ ] Add versioned commands/events for secret creation, rotation, grant,
      import, binding, revocation, purge, lease issuance, and reconciliation
      containing stable IDs and no values.
    - [ ] Use transactional outboxes/inboxes and stable idempotency identities
      without serializing plaintext into their payloads.
    - [ ] Ensure database rollback, publisher retry, and duplicate delivery
      cannot duplicate versions, grants, imports, leases, or raw
      materialization.
  - [ ] **Audit every privileged transition**
    - [ ] Record requester, mediator, runtime, source and target objects,
      permission, delivery mode, authorization-model version, decision,
      request/command ID, and outcome.
    - [ ] Distinguish metadata inspection, value submission, delegation,
      brokered use, and raw receipt without logging values.
    - [ ] Reauthorize live subscriptions before sending secret metadata,
      binding, rotation, or revocation events.
  - [ ] **Add redacted telemetry**
    - [ ] Measure version age, rotations, denied resolutions, active leases,
      broker operations, raw-delivery runs, revocation latency, and cleanup
      failures using opaque IDs.
    - [ ] Add automated sentinel scanning across structured logs, metrics,
      traces, NATS streams, PostgreSQL non-ciphertext columns, and test
      artifacts.

- [ ] **9. Add organization, project, repository, and agent UI**
  - [ ] **Manage owned secrets**
    - [ ] Add organization and project secret settings with metadata listing,
      write-only create/replace forms, rotation, disable, revoke, and purge
      controls gated by exact permissions.
    - [ ] Never prepopulate, reveal, return, or place values in LiveView state,
      HTML, client logs, flash messages, URL parameters, or reconnect payloads.
    - [ ] Show owner, status, version age, grants, affected imports/bindings,
      delivery modes, and last-use audit without displaying values.
  - [ ] **Manage grants and imports**
    - [ ] Add exact project/repository grant flows with mode, phase,
      destination, expiration, and scope review.
    - [ ] Add target-side import acceptance and alias selection, clearly
      distinguishing direct organization imports from project-owned secrets.
    - [ ] Show that imports are live references, are non-transitive, and stop
      working when their source grant or secret is revoked.
  - [ ] **Bind agents safely**
    - [ ] Generate instance binding forms from release slot declarations and
      list only imports eligible for the exact target scope.
    - [ ] Show selected attachments, phases, destinations, and whether the
      guest receives raw material or only a broker capability.
    - [ ] Require an explicit high-visibility confirmation for raw binding and
      explain that the guest can copy the value.
    - [ ] Show missing or revoked required bindings as authorization-safe
      invalid revision states.
  - [ ] **Add LiveView and browser tests**
    - [ ] Test users who can create, rotate, grant, import, bind, or only view
      metadata and prove each UI omits forbidden controls and values.
    - [ ] Test organization-to-project, organization-to-repository, and
      project-to-repository workflows plus revocation propagation.
    - [ ] Scan rendered HTML, LiveView payloads, browser logs, screenshots, and
      traces for sentinel values.

- [ ] **10. Verify end-to-end security and recovery**
  - [ ] **Add a real-system scenario**
    - [ ] Create one organization secret and one project secret, grant and
      import them into project and repository scopes, and bind them to separate
      slots without the binding user seeing either value.
    - [ ] Run one raw-file slot and one brokered slot in a real libkrun guest
      and verify exact version, instance revision, attachment, and authorization
      provenance.
    - [ ] Rotate both secrets and prove later runs use new versions while
      historical runs retain only protected version IDs.
    - [ ] Revoke grants during active raw and brokered runs and verify the
      distinct cancellation and live-denial guarantees.
    - [ ] Verify cross-project, cross-repository, cross-agent, cross-run, and
      cross-organization attempts fail closed.
  - [ ] **Exercise failure recovery**
    - [ ] Crash before and after encryption, database commit, version
      activation, lease issuance, raw materialization, guest start, broker use,
      revocation, guest destruction, and purge.
    - [ ] Reconcile without duplicate versions, leaked plaintext, orphaned
      mounts, reusable credentials, or false revocation claims.
    - [ ] Restore encrypted backups with the correct key set and prove missing
      or wrong keys fail closed.
  - [ ] **Run repository quality gates**
    - [ ] Run `cargo fmt --all -- --check`.
    - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
    - [ ] Run `cargo test --workspace --all-features`.
    - [ ] Run `cargo doc --workspace --all-features --no-deps`.
    - [ ] Run real PostgreSQL, NATS, libkrun, broker, and Playwright suites.
    - [ ] Run `mix precommit` in `web/`.
    - [ ] Run Mélange drift detection, `melange doctor`, and OpenFGA
      compatibility fixtures.
    - [ ] Run secret-sentinel scans and `git diff --check`.

- [ ] **11. Document and hand off**
  - [ ] **Document the security model**
    - [ ] Document ownership, grants, imports, non-transitivity, instance
      bindings, exact-version resolution, and permission vocabulary.
    - [ ] Document the difference between human delegation without disclosure,
      raw guest receipt, and non-disclosing brokered use.
    - [ ] Document encryption/key custody, rotation, revocation, purge, backup,
      recovery, audit, and unavoidable raw-delivery limitations.
  - [ ] **Document operations**
    - [ ] Document key provisioning and rotation, unavailable-key recovery,
      secret/import/binding inspection, runtime cancellation, orphan cleanup,
      and emergency revocation.
    - [ ] Record migration versions, authorization-model and Mélange versions,
      encryption algorithms/key versions, fixture IDs, test counts, and
      sentinel-scan evidence.
    - [ ] Create focused todo tasks for deliberately deferred production KMS,
      external vault, transparent proxy, additional broker adapters, or
      long-lived credential renewal work.

## Completion evidence

Populate this section while the task is in `tasks/in-progress/`. Include exact
commands and results, migration and generated-authorization provenance,
encryption/key-provider versions, golden secret/version/import/binding/lease
IDs, real-libkrun and fake-upstream evidence, Playwright results, sentinel-scan
results, and links to deliberate follow-up tasks.
