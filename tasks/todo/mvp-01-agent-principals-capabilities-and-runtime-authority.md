# MVP 01: Agent capability requirements and instance permissions

Owner: unassigned

## Outcome

Let a released agent describe the Hephaestus resources and operations it needs,
and let an authorized user satisfy those requirements while creating or
reconfiguring an agent instance.

Instance setup presents the release requirements, lets the user select exact
resources, and requires an explicit permission grant for each selection. The
result is an immutable instance revision containing exact capability bindings.
Required bindings must be valid before the revision can run.

At dispatch, Hephaestus creates an immutable authorization snapshot and one
short-lived runtime session for the exact run. The runtime may perform only the
operations present in both its snapshot ceiling and live authorization. Every
privileged operation is RLS-constrained and auditable.

```text
release capability requirements
→ selected resources and explicit permissions
→ immutable instance revision bindings
→ dispatch-time authorization snapshot
→ short-lived runtime session
→ ceiling check + live permission check + RLS + audit
```

## Locked decisions

| Area | Decision |
| --- | --- |
| Release contract | A release declares stable symbolic capability slots, compatible resource kinds, required operations, optional operations, and whether the entire slot is required. |
| Grant ceiling | An instance may receive only operations declared by its release. A user may omit optional operations but cannot add undeclared ones. |
| Exact binding | Each configured slot resolves to an exact Hephaestus resource and an explicit operation set. Names, paths, or project membership do not imply a binding. |
| User authority | Creating an instance does not itself authorize the creator to grant resources. Every binding requires permission to grant the selected operations on the exact resource. |
| Revision history | Capability bindings belong to an immutable instance revision. Any binding or permission change creates a new revision. |
| Live revocation | A binding records the run's maximum authority. Current OpenFGA/Mélange authorization may deny an operation at any time. |
| Runtime principal | The agent instance is the durable principal. The runtime session authenticates one exact run of one immutable instance revision. |
| Runtime credential | Each runtime session has one opaque, random, short-lived bearer credential. PostgreSQL stores only its hash, and queued work never contains it. |
| Effective permission | Every privileged call must match the runtime session, snapshot ceiling, exact resource and operation, live authorization, and RLS policy. |
| Resource semantics | Permissions describe controlled Hephaestus operations rather than filesystem-style read/write access. |
| Git resources | A repository binding is a named, exact resource capability. It may grant only declared Git operations, ref globs, and write-path globs; a repository name, project membership, or attachment never grants ambient Git authority. |
| Git read boundary | Raw Git reads may be restricted by repository and ref, but not by path. Path-restricted reads require a distinct filtered content API or virtual repository and are not implied by sparse checkout. |
| State | Private persistent state is allocated from the release's state requirement and is not a user-selected external capability binding. |
| Secrets | Secret slots, imports, bindings, exact-version resolution, and delivery policy remain typed secret contracts. Their leases attach to the run's runtime session. |
| Setup experience | Parameters, state, secret slots, and capability slots appear in one instance requirements review while retaining their distinct storage and enforcement models. |

## Initial permission vocabulary

The first implementation must define a closed compatibility matrix between
resource kinds and semantic operations. It must cover the existing
repository, project, agent-instance, run, and state-volume operations required
by installed agents.

Initial repository operations should distinguish metadata/tree inspection,
Git read, ref creation, fast-forward ref update, force update, ref deletion,
tag creation/deletion, run triggering, and attachment management. A Git write
binding must carry independent ref glob and changed-path glob constraints;
delete authority is never inferred from write authority. The initial Git
capability may constrain writes by path at receive time, but must not claim to
hide paths from a raw Git clone or fetch.

Initial project and agent-instance operations should distinguish inspection
from configuration, execution, update, pause, and recovery. Permission to
manage an instance must not imply permission to grant it more authority,
manage secrets, delete its project, or bypass approval for controlled
publication.

Mailbox, ingress, model, and outbound resource kinds will use this same
capability contract in their respective MVP tasks.

## Dependencies

- [`reusable-agent-releases-and-instances.md`](../done/reusable-agent-releases-and-instances.md)
- [`manage-delegate-and-deliver-secrets.md`](../done/manage-delegate-and-deliver-secrets.md)
- [`define-own-the-loop-agent-platform.md`](define-own-the-loop-agent-platform.md)

## Non-goals

This task does not implement mailboxes, public ingress, model providers,
outbound adapters, an Operator Agent, a Project Agent, human-to-agent
delegation, one-shot approvals, schedules, or long-lived service sessions.

It does not expose arbitrary database operations, host paths, plaintext
secrets, a generic “full project access” bit, or broad reusable Git
credentials. Git-specific runtime credential transport and Git HTTP enforcement
are implemented by MVP 01.1.

## Implementation checklist

- [ ] **1. Define capability requirements**
  - [ ] **Add provider-neutral domain types**
    - [ ] Add validated capability slot keys, resource kinds, semantic
      operations, requirement IDs, binding IDs, authorization-snapshot IDs,
      and runtime-session IDs.
    - [ ] Represent required and optional operations separately and validate
      that each operation is legal for its resource kind.
    - [ ] Define deterministic normalized forms, hashes, and idempotency keys
      for requirements, bindings, snapshots, sessions, and revocations.
    - [ ] Add tests for parsing, bounds, normalization, serialization,
      duplicate rejection, illegal resource-operation pairs, and deterministic
      identity.
  - [ ] **Extend release configuration**
    - [ ] Add capability declarations with a stable slot key, human-readable
      purpose, compatible resource kind, required operations, optional
      operations, and required/optional slot state.
    - [ ] Reject tenant resource IDs, resource names, permission grants, and
      bearer material in release source configuration.
    - [ ] Bind normalized declarations and their hash into the immutable
      release agent and release provenance.
    - [ ] Preserve valid releases that declare no capability slots.
    - [ ] Add parser and release-domain tests for valid declarations,
      unsupported versions, malformed operations, duplicate slots, normalized
      hashes, and immutable publication.
  - [ ] **Define repository capability requirements**
    - [ ] Define repository operations and normalized ref/path glob grammar,
      including explicit create, update, force-update, delete, and tag rules.
    - [ ] Require a release to declare each named repository slot and its
      maximum operation/ref/path ceiling; reject resource names, remote URLs,
      token values, and tenant identifiers in release source.
    - [ ] Specify receive-time changed-path semantics for additions, deletions,
      renames, merges, and new refs, including byte/object limits and
      deny-by-default behavior for ambiguous history.
    - [ ] State and test that raw Git read policy is repository/ref scoped;
      path-restricted reads are a later filtered-content capability.

- [ ] **2. Bind instance permissions**
  - [ ] **Persist immutable revision bindings**
    - [ ] Add capability requirement and immutable instance-revision binding
      records with exact resource type, resource ID, granted operation set,
      creator, authorization-model version, and creation time.
    - [ ] Enforce complete foreign keys or equivalent typed integrity for every
      supported resource kind.
    - [ ] Prevent cross-project bindings unless the resource kind has an
      explicit authorized sharing contract.
    - [ ] Reject undeclared slots, incompatible resources, missing required
      operations, undeclared optional operations, duplicates, and stale
      revision updates.
    - [ ] Mark a candidate revision visibly unrunnable when a required binding
      is missing, revoked, unavailable, or invalid under current platform
      policy.
    - [ ] Make every binding or permission change create a new immutable
      revision and preserve all historical bindings referenced by runs.
  - [ ] **Authorize grants**
    - [ ] Define the exact user permission required to grant each semantic
      operation on each supported resource kind.
    - [ ] Check resource selection, grant authority, tenant scope, release
      requirement, operation ceiling, and platform policy in one transaction.
    - [ ] Require independent authorization for every resource in a
      multi-binding instance revision.
    - [ ] Ensure permission to create, configure, or execute an instance does
      not implicitly authorize resource grants.
    - [ ] Add real-PostgreSQL tests for valid grants, partial authority,
      cross-project denial, operation broadening, concurrent revisions,
      revocation, and historical retention.

- [ ] **3. Make agent instances authorization subjects**
  - [ ] **Extend the canonical authorization model**
    - [ ] Add explicit agent-instance relations for every supported resource
      and semantic operation.
    - [ ] Define separate user permissions for inspecting a resource, using
      it, and granting an agent access to it.
    - [ ] Ensure organization or project membership grants no ambient
      agent-instance authority.
    - [ ] Define authoritative domain records that produce every new
      `melange_tuple`.
    - [ ] Regenerate and commit specialized Mélange SQL with the
      repository-pinned CLI.
    - [ ] Extend OpenFGA/Mélange compatibility fixtures and unknown-object
      deny-by-default tests.
  - [ ] **Apply PostgreSQL RLS**
    - [ ] Add forced RLS policies for capability requirements, bindings,
      snapshots, runtime sessions, and capability audit records.
    - [ ] Generalize transaction-local context to carry the effective agent
      instance, exact run, runtime session, and request ID.
    - [ ] Keep agent-facing transactions on a non-`BYPASSRLS` role and prevent
      callers from selecting a trusted worker identity.
    - [ ] Add real-PostgreSQL tests for user, agent-instance, exact-run, and
      trusted-worker flows, including forged and incomplete context.

- [ ] **4. Snapshot authority and issue runtime sessions**
  - [ ] **Resolve authority at dispatch**
    - [ ] Reauthorize the exact instance, active revision, run, attachment,
      release use, capability bindings, secret bindings, selected resources,
      and lifecycle state immediately before dispatch.
    - [ ] Persist one immutable authorization snapshot containing the exact
      revision binding IDs, granted operations, resource IDs, secret lease
      identities, authorization-model version, and deterministic snapshot
      hash.
    - [ ] Create one runtime session bound to the exact instance, revision,
      run, optional attachment, snapshot, issue time, expiry, and lifecycle
      state.
    - [ ] Mint one fresh opaque credential, store only its hash, and deliver
      bearer material only through the runtime bootstrap channel.
    - [ ] Make dispatch retry and NATS redelivery idempotent without creating
      conflicting active sessions or bearer credentials.
    - [ ] Deny dispatch with stable diagnostics when any required binding is
      missing, revoked, unauthorized, or incompatible.
  - [ ] **Attach runtime leases**
    - [ ] Associate exact secret leases and future capability-specific leases
      with the same runtime session and authorization snapshot.
    - [ ] Preserve the distinct raw-secret and brokered-secret permissions and
      delivery behavior.
    - [ ] Define session expiry, revocation, cancellation, terminal cleanup,
      and crash reconciliation.
    - [ ] Add dispatch tests for mixed ordinary and secret capabilities,
      rotation, revocation, concurrent dispatch, retries, and stale revisions.

- [ ] **5. Authorize privileged runtime calls**
  - [ ] **Authenticate and attenuate each request**
    - [ ] Authenticate the opaque credential and match its exact runtime
      session, run, instance, revision, expiry, and active lifecycle.
    - [ ] Resolve the requested semantic operation and exact resource to one
      binding in the immutable snapshot.
    - [ ] Reject any request outside the snapshot ceiling before invoking
      application or database services.
    - [ ] Check current OpenFGA/Mélange permission for the agent instance and
      resource inside the RLS-constrained transaction.
    - [ ] Recheck capability-specific live state such as binding revocation,
      attachment status, release use, secret lease, or resource lifecycle.
    - [ ] Add tests for credential theft across runs, instances, revisions,
      resources, operations, attachments, expiry, and revoked bindings.
  - [ ] **Handle revocation honestly**
    - [ ] Deny new API and broker calls immediately after live authorization
      or a bound resource is revoked.
    - [ ] Request cancellation when revoked authority has already materialized
      a sensitive guest resource that cannot be withdrawn through an API
      check.
    - [ ] Record when a read-only mount or raw secret may already have been
      observed and cannot be retroactively revoked.
    - [ ] Prevent session renewal, retry, or worker recovery from broadening
      the immutable snapshot.
    - [ ] Add race tests before dispatch, after session creation, during guest
      provisioning, during a privileged call, and after revocation.
  - [ ] **Specialize runtime authority for Git**
    - [ ] Define the authenticated runtime-Git principal as the exact runtime
      session, never as a human user or a reusable agent-wide identity.
    - [ ] Require every Git request to recheck credential validity, exact run,
      binding, operation, repository, ref/path constraints, expiry, current
      authorization, and resource lifecycle.
    - [ ] Delegate Git credential format, Git HTTP authentication, ref
      advertisement, receive enforcement, and runtime worktree delivery to
      MVP 01.1 without weakening this task's immutable binding ceiling.

- [ ] **6. Add the instance permission UI**
  - [ ] **Review requirements during instance creation**
    - [ ] Present ordinary parameters, private state, secret slots, and
      capability slots in one release requirements flow.
    - [ ] For each capability slot, show its purpose, resource kind, required
      operations, optional operations, and whether the slot is required.
    - [ ] List only exact resources the current user may both select and grant
      with the requested operation set.
    - [ ] Require explicit resource selection and permission confirmation for
      every required capability.
    - [ ] Show why the instance is unrunnable when a required resource or
      permission is unavailable.
  - [ ] **Manage revisions and permission changes**
    - [ ] Show active revision bindings as resource, operation set, grantor,
      live status, and last-use metadata without exposing secrets.
    - [ ] Create a new revision for resource replacement, optional-operation
      changes, or binding removal.
    - [ ] Show a permission diff for release updates and require explicit
      approval for every newly required resource or operation.
    - [ ] Distinguish release-required operations, optional grantable
      operations, current grants, and live revocations.
    - [ ] Add LiveView and browser tests for complete configuration, partial
      grant authority, denied resources, optional permissions, invalid
      revisions, update diffs, revocation, and authorization-safe live refresh.

- [ ] **7. Audit, inspect, and observe**
  - [ ] **Record complete authority provenance**
    - [ ] Record requester, grantor, agent instance, revision, run, runtime
      session, authorization snapshot, binding, permission, resource,
      authorization-model version, request ID, decision, and outcome.
    - [ ] Keep bearer credentials, secret values, sensitive parameters, request
      bodies, and provider authorization material out of audit and telemetry.
    - [ ] Add read-only inspection for an instance's declared requirements,
      bound resources, granted operations, live status, snapshot ceiling,
      runtime sessions, denials, and revocation effects.
    - [ ] Reauthorize live subscriptions before publishing permission,
      session, audit, or denial updates.
    - [ ] Measure session issuance, expiry, revocation latency, capability
      calls, ceiling denials, live-authorization denials, and invalid revisions
      using bounded opaque labels.

- [ ] **8. Verify and document**
  - [ ] Document the release declaration, instance binding, user grant,
    revision, snapshot, runtime session, live check, revocation, and audit
    lifecycle.
  - [ ] Document the supported resource-operation matrix and the exact user
    authority required to grant each operation.
  - [ ] Document the instance creation and permission-update UI.
  - [ ] Run `cargo fmt --all -- --check`.
  - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
  - [ ] Run `cargo test --workspace --all-features`.
  - [ ] Run `cargo doc --workspace --all-features --no-deps`.
  - [ ] Run the real-PostgreSQL authorization, capability, revision,
    dispatch, runtime-session, revocation, and RLS suites.
  - [ ] Run Mélange generated-migration drift detection, `melange doctor`, and
    OpenFGA compatibility fixtures.
  - [ ] Run `mix precommit` in `web/`.
  - [ ] Run the capability configuration and permission-diff Playwright
    scenarios.
  - [ ] Run secret and runtime-credential sentinel scans.
  - [ ] Run `git diff --check`.

## Completion evidence

Record the configuration and schema versions, supported resource-operation
matrix, migration IDs, authorization-model and generated-SQL hashes, golden
release requirement and instance-binding IDs, authorization snapshots,
runtime sessions, UI screenshots, grant and denial fixtures, revocation-race
results, test counts, sentinel-scan results, and deliberate follow-up tasks.
