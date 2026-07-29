# Reusable agent releases, project instances, and exact runs

Owner: unassigned

## Outcome

Replace the current project-scoped name-only agent record and direct
push-to-agent-run coupling with a reusable software model:

```text
agent source commit
→ isolated build
→ immutable release
→ exported release agent
→ project-owned agent instance
→ immutable instance revision
→ repository/ref attachment
→ run against an exact target commit
```

An agent implementation can live in one repository, publish built releases,
and be imported into many projects. Each consuming project receives an
independent instance with typed parameters, authorization, persistent state,
attachments, revision history, and runs.

## Locked decisions

| Area | Decision |
| --- | --- |
| Release identity | Releases are immutable and bind an exact source repository, commit, build definition, build run, and artifact-manifest hash. |
| Agent lineage | A source-repository-owned agent family gives one exported agent a stable identity across releases. A display name or exported key alone never establishes update compatibility. Forking or copying the source into another repository creates a different family that must be imported as a separate instance. |
| Runtime identity | An agent instance is a durable logical installation in one project, not a VM and not a branch. |
| Reuse | A published release may export an agent that instances in other authorized projects can import. |
| Release access | Existing instances retain their configuration and history after source access is revoked, but every release-artifact acquisition for a new normal or update guest reauthorizes current source access. A host cache never bypasses that check. An already materialized guest may finish; destroying it destroys that runnable copy. |
| Configuration history | An immutable agent-instance revision resolves one release agent, ordinary parameters, secret references, project-selectable resource sizing and network restrictions, and the resulting effective runtime policy. |
| Execution identity | Every run binds an exact instance revision, attachment, target repository, target ref, target commit, and triggering receive or command. |
| State | Persistent state belongs to the agent instance and may be shared deliberately across that instance's attachments. Separate instances provide state isolation. |
| POC state compatibility | A release update for a stateful instance requires the same state capability and an update hook. Changing between stateless and stateful, or updating a stateful instance without a hook, produces a durable visible invalid candidate and never starts the update guest. Parameter-only revisions on the same release do not require a hook. |
| Attachments | An instance may attach only to repositories belonging to its project. Attachments carry ref selection and trigger policy. |
| Historical identity | Removing an attachment or revoking a release creates a tombstone. Records referenced by revisions, updates, or runs are never hard-deleted. |
| Built code | Runtime guests execute built artifacts from an immutable read-only release mount. They never build implicitly during a normal agent run. |
| Runtime policy ownership | The release owns its executable, arguments, working directory, pinned runtime root image, mount contract, state requirement, network ceiling, and resource bounds. A consuming project may select resources within those bounds and restrict networking, but cannot replace release-owned fields or broaden capabilities. Platform policy participates in revision validation, and the fully resolved policy and platform-policy version are immutable revision provenance. A later policy change may deny a new guest but never silently mutate its revision or substitute another image or capability set. |
| Updates | A newly published source release does not mutate consumers automatically. An authorized update creates and activates a new instance revision. |
| Run/update exclusion | Starting an update atomically closes the instance run gate. Existing normal requests and runs drain before migration; new matching triggers are recorded durably and deferred without binding a revision. No normal dispatcher may start work while migration or recovery owns the gate. |
| Update hook | Candidate-release update hooks execute inside an isolated guest with an exclusive instance-volume lease. No guest hook ever executes in a host shell. |
| Rollback responsibility | The agent owns migration transactions, idempotency, cleanup, and safe rollback. Hephaestus coordinates leases, execution, audit, and revision activation only. |
| Hook exit zero | The state volume is compatible with the candidate revision, which must be activated. Durable hook success is an irreversible commit point; activation failure pauses the instance for activation recovery and never restores the current revision as runnable. |
| Hook explicit nonzero | The agent guarantees rollback completed and the volume remains compatible with the current revision; the candidate is rejected. |
| Hook abnormal failure | Timeout, signal, VM loss, or protocol loss leaves compatibility unknown; the current revision remains selected and the instance is paused for recovery. |
| Canonical writes | Build and runtime guests cannot mutate canonical Git storage or release storage. Host-side controlled importers perform all publication. |
| Compatibility | This POC may rewrite existing migrations and domain APIs. No backward-compatible migration path for the current hollow `agents` records is required. |

## Non-goals

This task does not add guest Git credentials, guest-originated pushes,
long-lived interactive VMs, automatic release-channel upgrades, host-managed
agent-state rollback, arbitrary host shell hooks, public release marketplaces,
or plaintext secret delivery. Secret bindings remain typed references until
[`manage-delegate-and-deliver-secrets.md`](manage-delegate-and-deliver-secrets.md)
is completed.

Supporting several exported agents in one release should be represented in the
schema, but the first configuration format may continue to expose one root
agent definition per source commit.

General state-capability transitions and declared hookless state compatibility
are deferred to
[`support-agent-state-capability-transitions.md`](support-agent-state-capability-transitions.md).

## Dependencies and affected boundaries

The work builds on exact-commit materialization, libkrun execution, persistent
state volumes, safe workspace sealing/import, PostgreSQL/Mélange authorization,
transactional outboxes, and the Phoenix review UI. It affects the forge receive
flow, agent configuration, run orchestration, workspace mounts, authorization
model, migrations, NATS commands, and project/repository UI.

## Implementation checklist

- [ ] **1. Finalize terminology and provider-neutral domain contracts**
  - [ ] **Define stable identifiers and value objects**
    - [ ] Add provider-neutral `ReleaseId`, `ReleaseArtifactId`,
      `AgentFamilyId`, `ReleaseAgentId`, `BuildRequestId`, `AgentInstanceId`,
      `AgentInstanceRevisionId`, `AgentAttachmentId`, and `AgentUpdateId`
      types.
    - [ ] Keep `RunId`, `RepositoryId`, `ProjectId`, commit IDs, refs, and
      configuration hashes as exact typed values at service boundaries.
    - [ ] Validate opaque IDs, release versions, agent keys, parameter names,
      artifact paths, and ref selectors without accepting filesystem-derived
      identity.
  - [ ] **Define aggregate contracts**
    - [ ] Define immutable release, release-artifact, and exported-agent
      domain models.
    - [ ] Define the source-repository-owned agent family that identifies one
      exported agent across releases independently of display names and keys
      from other repositories.
    - [ ] Define project-owned agent-instance and immutable
      agent-instance-revision models.
    - [ ] Define repository/ref attachment and trigger-policy models.
    - [ ] Define build-request, instance-update, and exact run-provenance
      models.
    - [ ] Document that `VmInstance` remains ephemeral and is never the
      product-level `AgentInstance`.
  - [ ] **Add domain tests**
    - [ ] Test identifier parsing, serialization, malformed input rejection,
      and path/ref validation.
    - [ ] Test valid and invalid lifecycle transitions for releases,
      instances, instance revisions, attachments, builds, and updates.
    - [ ] Test that update compatibility requires the same agent family and
      cannot be established by reusing an exported key in another repository.
    - [ ] Test deterministic hashes and idempotency keys for builds, releases,
      instance revisions, attachments, updates, and runs.

- [ ] **2. Replace the PostgreSQL agent and release data model**
  - [ ] **Add release and build records**
    - [ ] Add source-repository-owned `agent_families` with stable IDs and
      repository-scoped exported keys.
    - [ ] Add repositories' immutable `releases` records with source commit,
      source ref provenance, build-definition hash, build-run ID, manifest
      hash, lifecycle state, publication actor, and timestamps.
    - [ ] Add `release_artifacts` with normalized relative paths, kinds, modes,
      content hashes, sizes, media types, storage keys, and provenance.
    - [ ] Add `release_agents` linking an agent family, stable exported key,
      and runtime configuration to a release.
    - [ ] Add durable build requests, build runs or run-kind metadata, build
      diagnostics, and idempotent build-command/outbox records.
    - [ ] Enforce immutable published-release fields and repository-scoped
      uniqueness for release names or versions.
  - [ ] **Replace hollow agents with project instances**
    - [ ] Replace or rewrite `agents` as project-owned `agent_instances` with
      stable name, lifecycle state, active revision, state-volume ownership,
      and timestamps.
    - [ ] Add immutable `agent_instance_revisions` resolving release agent,
      parameter document, parameter hash, secret references, resource policy,
      project network restriction, fully resolved effective runtime policy,
      platform-policy version, creator, and timestamps.
    - [ ] Add `agent_attachments` connecting an instance to a repository in
      the same project, with ref selector, trigger policy, enabled state, and
      removal tombstone and audit timestamps.
    - [ ] Repoint state volumes, runs, run requests, review provenance, and
      audit records to the instance and exact instance revision.
    - [ ] Add database constraints preventing cross-project repository
      attachments and cross-instance revision activation.
  - [ ] **Add update state**
    - [ ] Add durable agent-update records containing current and candidate
      revisions/releases, stable update ID, actor, state, hook run, exit
      result, timestamps, diagnostics, and final decision.
    - [ ] Add lifecycle states for updating, update rejection, unknown-state
      pause, activation-recovery pause, recovery, and successful activation.
    - [ ] Add uniqueness and compare-and-swap constraints preventing concurrent
      updates or activation from a stale current revision.
    - [ ] Add a transactional instance run gate, durable deferred triggers, and
      constraints preventing migration from starting while a normal request,
      run, or instance-volume lease remains active.
  - [ ] **Add indexes and triggers**
    - [ ] Index releases by repository and publication state.
    - [ ] Index exported agents by release and stable agent key.
    - [ ] Index instances by project and lifecycle state.
    - [ ] Index attachments by target repository/ref and enabled state.
    - [ ] Index instance revisions and updates by instance and creation time.
    - [ ] Update UI wakeup triggers for projects, releases, instances,
      attachments, updates, and runs.
  - [ ] **Add real-PostgreSQL persistence tests**
    - [ ] Test release immutability and artifact uniqueness.
    - [ ] Test cross-project imports with authorized release access.
    - [ ] Test same release imported into separate projects produces isolated
      instances and volumes.
    - [ ] Test attachment project-boundary constraints.
    - [ ] Test active-revision CAS and concurrent-update rejection.
    - [ ] Test attachment and release tombstones preserve every historical
      foreign-key target.
    - [ ] Test run-gate closure, draining, and deferred-trigger creation under
      concurrent receives and dispatch.
    - [ ] Test immediate transaction visibility for instance, attachment, and
      update state changes.

- [ ] **3. Extend the source configuration and typed parameter model**
  - [ ] **Version the configuration schema**
    - [ ] Add build command, arguments, working directory, build root image,
      resource limits, network profile, declared artifact outputs, and build
      triggers.
    - [ ] Define release runtime command paths relative to the immutable
      release mount rather than arbitrary host paths.
    - [ ] Bind the exact runtime executable, arguments, working directory,
      pinned root-image digest, mount contract, state requirement, network
      ceiling, and resource bounds into the immutable release agent.
    - [ ] Define project-selectable resource sizing within release bounds and
      networking as a restriction of the release ceiling.
    - [ ] Resolve and persist the exact effective runtime policy and
      platform-policy version when validating an instance revision.
    - [ ] Reject consumer attempts to replace release-owned runtime fields,
      broaden network access, exceed resource bounds, or use a root image the
      platform does not allow; never substitute another image silently.
    - [ ] Define a stable exported agent key separately from its mutable
      display name.
    - [ ] Add optional candidate-release update-hook command, shell contract,
      arguments, timeout, and resource limits.
    - [ ] Persist whether the exported agent requires instance state as part of
      its release-agent contract.
    - [ ] Preserve strict version rejection and structured diagnostics for
      unsupported configuration versions.
  - [ ] **Define typed parameters**
    - [ ] Support bounded string, integer, boolean, and enum parameter schemas
      with required/default metadata.
    - [ ] Reject duplicate, malformed, unbounded, or reserved parameter names.
    - [ ] Validate instance parameter values completely before creating an
      instance revision.
    - [ ] Canonicalize validated parameter JSON and persist a deterministic
      parameter hash.
    - [ ] Keep host-generated run context outside user-configurable
      parameters.
    - [ ] Represent secrets only as typed references and reject a runnable
      revision when required bindings cannot be resolved safely.
  - [ ] **Add parser and compatibility tests**
    - [ ] Test configuration versioning, build definitions, artifact paths,
      runtime commands, parameter types/defaults, and update-hook validation.
    - [ ] Test deterministic effective-policy resolution across release
      requirements, project selections, and platform restrictions.
    - [ ] Test that project configuration cannot replace the runtime command,
      arguments, working directory, root image, mounts, or state requirement,
      and cannot broaden networking.
    - [ ] Test new-release parameter compatibility with existing instance
      values.
    - [ ] Produce stable structured diagnostics for a release update that
      changes state capability or omits the hook required by a stateful
      instance.
    - [ ] Test diagnostic stability for removed, renamed, newly required, and
      type-changed parameters.
    - [ ] Test deterministic normalized configuration and parameter hashes.

- [ ] **4. Implement isolated builds and immutable release artifacts**
  - [ ] **Create the build workflow**
    - [ ] Convert an accepted source push or authorized manual command into an
      idempotent build request for an exact repository, commit, ref, and
      build-definition hash.
    - [ ] Materialize the exact source tree read-only and a separate writable
      build workspace.
    - [ ] Run the build command in a resource-limited microVM using the pinned
      build root image and declared network policy.
    - [ ] Prevent the build guest from receiving canonical Git credentials,
      release-store write access, instance state volumes, or host paths.
    - [ ] Capture build logs, metrics, exit results, diagnostics, and declared
      artifact outputs durably.
  - [ ] **Seal and import build outputs**
    - [ ] Treat build completion as one-way finalization, stop/reap the VM,
      atomically seal the build workspace, and import only from the sealed
      path.
    - [ ] Reuse or extend the safe importer to reject symlinks, devices, FIFOs,
      sockets, escapes, unsupported modes, quota violations, and undeclared
      outputs.
    - [ ] Construct a deterministic release artifact manifest with hashes,
      sizes, paths, modes, media types, and complete build provenance.
    - [ ] Store artifacts under an opaque-ID/content-addressed canonical
      layout that cannot be influenced by repository paths.
    - [ ] Ensure retries reuse durable build identity and never create
      conflicting release artifacts.
  - [ ] **Publish releases transactionally**
    - [ ] Create a draft release only after the build result and complete
      artifact manifest are durable.
    - [ ] Add an authorized explicit publish command that freezes the release
      and emits its outbox event.
    - [ ] Prevent failed, incomplete, rejected, or already published releases
      from being mutated or republished inconsistently.
    - [ ] Retain source commit, build run, normalized configuration, exported
      agent, and artifact hashes as immutable provenance.
  - [ ] **Add build and release tests**
    - [ ] Test exact-commit builds for two branches with different contents.
    - [ ] Test artifact import rejection for every unsafe filesystem type and
      quota boundary.
    - [ ] Test build-request and release-publication idempotency.
    - [ ] Test published release immutability.
    - [ ] Test a built executable runs from the imported read-only release
      tree rather than the source tree.

- [ ] **5. Implement project imports, instances, revisions, and attachments**
  - [ ] **Import a released agent**
    - [ ] Add an authorized command that selects an accessible published
      release agent and creates a project-owned instance.
    - [ ] Validate initial parameters and secret references against the
      release-agent schema.
    - [ ] Create the initial immutable instance revision and activate it
      atomically with instance creation.
    - [ ] Allocate one persistent state volume per instance only when its
      released agent requests state.
    - [ ] Make repeated import commands idempotent without conflating imports
      in different projects.
  - [ ] **Manage instance revisions**
    - [ ] Add authorized parameter, secret-binding, in-bounds resource-sizing,
      and network-restriction update commands that create new immutable
      revisions.
    - [ ] Resolve and persist the effective policy from immutable release-owned
      runtime fields, project selections, and current platform constraints.
    - [ ] Require a forked or copied implementation to publish its own agent
      family and be imported as a separate instance rather than updating an
      instance from the original family.
    - [ ] Never rewrite revisions referenced by an update or run.
    - [ ] Validate a candidate revision completely before it may become active.
    - [ ] Persist unsupported state-capability changes and hookless stateful
      release updates as visible invalid candidates without starting a guest.
    - [ ] Record activation and rejection as durable instance events.
  - [ ] **Attach instances to target repositories**
    - [ ] Add authorized create, enable, disable, update, and remove attachment
      commands.
    - [ ] Implement removal as a tombstone so historical run provenance keeps
      resolving the exact attachment.
    - [ ] Validate that the target repository belongs to the instance project.
    - [ ] Compile and validate exact refs or bounded ref-prefix selectors.
    - [ ] Keep trigger policy on the attachment rather than the imported
      agent's source branch.
    - [ ] Permit one instance to share its state across several attachments
      while making separate instances the explicit isolation mechanism.
  - [ ] **Add service tests**
    - [ ] Test importing one release into several projects with different
      parameters and independent state.
    - [ ] Test attaching one instance to several repositories in its project.
    - [ ] Test cross-project attachment denial.
    - [ ] Test parameter revision history and active-revision immutability.
    - [ ] Test allowed resource and network restrictions and reject every
      attempted release-owned runtime override.
    - [ ] Test that a release from a forked repository cannot update an
      instance of the original repository's family even when its key matches.
    - [ ] Test disabled and removed attachments cannot trigger new runs.
    - [ ] Test invalid update candidates expose stable state-compatibility
      diagnostics and leave the active revision and volume untouched.

- [ ] **6. Bind normal runs to releases, revisions, attachments, and targets**
  - [ ] **Replace direct source-config run creation**
    - [ ] Stop treating a target repository's `agent.toml` as an implicit
      project instance that directly runs on push.
    - [ ] On an accepted target push, resolve enabled attachments whose ref
      selector matches each accepted update.
    - [ ] When an instance run gate is closed, record each matching receive or
      command as one durable deferred trigger without selecting an instance
      revision.
    - [ ] Create one idempotent run request per exact attachment, active
      instance revision, target repository, target ref, target commit, and
      receive ID.
    - [ ] Recheck instance lifecycle, active revision, attachment state,
      authorization, and release availability in the same transaction that
      creates the durable run request.
    - [ ] On reopening the gate, re-evaluate and materialize deferred triggers
      idempotently against the then-active revision, or retain a durable denial
      diagnostic.
  - [ ] **Construct the runtime filesystem contract**
    - [ ] Mount the exact release artifact tree read-only at `/release`.
    - [ ] Mount the exact target source tree read-only at `/workspace/repo`.
    - [ ] Provide a separate writable result workspace at `/workspace/work`.
    - [ ] Mount the instance's persistent state volume exclusively at
      `/var/lib/hephaestus` when requested.
    - [ ] Write canonical validated ordinary parameters to a read-only
      `/run/hephaestus/parameters.json`.
    - [ ] Provide host-generated context separately with immutable instance,
      revision, release, attachment, repository, ref, commit, run, and mount
      identifiers.
    - [ ] Execute only the released runtime command under the existing
      unprivileged guest identity and host-constrained policy.
    - [ ] Enforce the revision's immutable resolved resource and network policy,
      including the release ceiling, project restriction, and platform policy
      recorded when the revision was validated.
    - [ ] Revalidate that the immutable resolved policy remains allowed before
      launch; if platform policy has tightened, deny the guest instead of
      silently running it under a different effective policy.
    - [ ] Reauthorize release use before every logical artifact acquisition and
      VM start even when the exact artifacts already exist in a host cache.
    - [ ] Allow an already materialized and started guest to finish if source
      access is later revoked; never reuse its runnable filesystem for another
      run after that guest is destroyed.
  - [ ] **Preserve controlled result publication**
    - [ ] Bind result provenance to the exact instance revision and release
      artifact manifest in addition to the target commit.
    - [ ] Publish result commits only into the target repository through the
      host-side result publisher.
    - [ ] Keep exact target input commit as result parent and preserve CAS
      approval behavior.
  - [ ] **Add run tests**
    - [ ] Test that identical target commits run differently under two
      instance revisions with different releases or parameters.
    - [ ] Test that historical runs retain their exact release and parameter
      provenance after an instance update.
    - [ ] Test shared state across attachments of one instance and isolation
      between two instances.
    - [ ] Test release, source, work, runtime-parameter, and state mount
      permissions inside a real libkrun guest.
    - [ ] Test duplicate receives and NATS redelivery never launch duplicate
      runs.
    - [ ] Test deferred triggers bind only the revision active after the gate
      reopens and cannot race an update hook.
    - [ ] Test source-access revocation blocks the next guest start despite a
      warm host artifact cache while an already started guest may finish.
    - [ ] Test a later platform-policy change either still permits the exact
      resolved revision or denies launch without silently changing execution.

- [ ] **7. Implement safe agent-instance updates**
  - [ ] **Create candidate updates**
    - [ ] Add an authorized update command selecting a published release agent
      from the instance's exact agent family.
    - [ ] Resolve and validate candidate parameters, secret references,
      in-bounds resource sizing, network restriction, and the release-owned
      runtime contract into a candidate instance revision.
    - [ ] Persist a stable update ID and compare-and-swap expectation for the
      currently active revision.
    - [ ] Reject concurrent updates, stale candidates, revoked releases, and
      updates for paused/recovering instances unless explicitly allowed.
    - [ ] Reject state-capability changes and hookless release updates for
      stateful instances as visible unsupported candidates before closing the
      run gate or acquiring the volume.
  - [ ] **Run the update hook in isolation**
    - [ ] In the update-creation transaction, CAS-close the instance run gate
      and begin durably deferring new matching triggers.
    - [ ] Drain every pre-gate normal request and run, then prove no normal
      request, run, or lease remains before entering migration.
    - [ ] Block all normal dispatch and acquire the instance state volume under
      an exclusive fenced lease.
    - [ ] Boot a special update run using the candidate release and its
      constrained update-hook shell inside the guest.
    - [ ] Mount candidate and previous releases read-only and expose old/new
      validated parameter documents.
    - [ ] Pass stable update, current/candidate revision, and
      current/candidate release identifiers as host-generated context.
    - [ ] Enforce update-specific resource, body/output, and wall-clock limits.
    - [ ] Persist hook logs, metrics, lifecycle events, exit status, and
      diagnostics without publishing a repository result branch.
  - [ ] **Apply the exit contract**
    - [ ] On exit zero, durably mark the hook successful and CAS-activate the
      candidate revision before re-enabling normal runs.
    - [ ] Treat durable hook success as the irreversible commit point:
      subsequent candidate revocation or authorization change cannot return
      the old revision to service, and activation failure pauses the instance
      for activation recovery.
    - [ ] On explicit nonzero exit, record agent-declared rollback completion,
      reject the candidate, retain the current revision, and restore the
      instance's previous runnable lifecycle state.
    - [ ] On timeout, signal, VM failure, lost protocol, or host uncertainty,
      retain the current revision but pause the instance as state compatibility
      unknown.
    - [ ] Never claim that Hephaestus rolled back agent-owned state.
    - [ ] Release the fenced volume lease and clean the update VM/runtime
      resources on every terminal path.
    - [ ] Reopen the run gate and materialize deferred triggers only after
      successful activation or an explicit nonzero result that safely restores
      the current revision; keep the gate closed on every uncertain or
      activation-recovery path.
  - [ ] **Reconcile crashes and retries**
    - [ ] Deliver a stable `HEPHAESTUS_UPDATE_ID` so agent hooks can implement
      idempotency.
    - [ ] Reconcile a host crash before hook start without creating a second
      logical update.
    - [ ] Reconcile a crash during the hook to the unknown-state paused path.
    - [ ] Reconcile a crash after durable hook success but before activation
      by activating that exact candidate without rerunning the migration.
    - [ ] Add explicit authorized recovery, retry, reject, and resume commands
      with durable audit events.
  - [ ] **Add update-hook tests**
    - [ ] Test successful migration activates the candidate release and
      preserves instance identity, attachments, and volume.
    - [ ] Test explicit nonzero exit keeps the current revision active.
    - [ ] Test timeout, signal, and VM loss pause the instance.
    - [ ] Test stale activation and concurrent update rejection.
    - [ ] Test that a CAS or invariant anomaly after durable hook success enters
      activation recovery and never runs the previous revision.
    - [ ] Test that source-access or release revocation after durable hook
      success cannot veto candidate activation but blocks its next guest start.
    - [ ] Test normal requests drain before migration, triggers received behind
      the closed gate are deferred, and no old-revision run starts afterward.
    - [ ] Test stable update IDs and crash reconciliation at each durable
      boundary.
    - [ ] Test an agent-owned transactional SQLite rollback followed by a
      successful normal run on the current release.

- [ ] **8. Extend authorization, RLS, and privileged audit**
  - [ ] **Update the canonical OpenFGA model**
    - [ ] Add build, release, release-agent use/import, instance management,
      attachment management, update, execute, and recovery permissions with
      relationship inheritance.
    - [ ] Keep the agent instance parented by its consuming project.
    - [ ] Make release visibility and import permission inherit from the source
      repository/project without copying grants into consuming projects.
    - [ ] Define release-use authorization for an installed instance so source
      access revocation blocks every subsequent logical artifact acquisition
      and guest start, independently of host caching.
    - [ ] Define target-repository permission requirements for attachment and
      result publication.
  - [ ] **Update Mélange inputs and generated SQL**
    - [ ] Extend `melange_tuples` from authoritative release, project,
      instance, and attachment relationships.
    - [ ] Run the repository-pinned Mélange CLI against the canonical `.fga`
      model.
    - [ ] Commit the actual generated specialized permission SQL as a reviewed
      versioned migration.
    - [ ] Update drift detection and `melange doctor` coverage.
  - [ ] **Apply PostgreSQL RLS**
    - [ ] Add `USING` and `WITH CHECK` policies for builds, releases,
      artifacts, release agents, instances, revisions, attachments, updates,
      and their audit metadata.
    - [ ] Ensure organization and project list queries filter imported agents
      and instances through RLS without manual tenant predicates.
    - [ ] Keep the application role unable to own protected tables or bypass
      RLS and force RLS where appropriate.
  - [ ] **Authorize every external side effect**
    - [ ] Check permission before build execution, release publication,
      artifact access, agent import, attachment changes, normal execution,
      update-hook execution, recovery, and result publication.
    - [ ] Recheck release use immediately before materializing each new normal
      or update guest; do not terminate a guest whose authorized artifact
      acquisition and start already completed.
    - [ ] Reauthorize live subscriptions before delivering release, instance,
      update, or run events.
    - [ ] Write structured privileged audit events with actor, permission,
      object, decision, request ID, and authorization-model version.
  - [ ] **Add authorization tests**
    - [ ] Test release import across authorized and unauthorized projects.
    - [ ] Test revoking source access leaves instance and run history visible
      but blocks new normal and update guests even with cached artifacts.
    - [ ] Test instance management, attachment, execution, update, and recovery
      decisions for owners, maintainers, members, and outsiders.
    - [ ] Test RLS list/read/insert/update/delete behavior and normal-role
      non-bypass.
    - [ ] Update OpenFGA/Mélange parity fixtures and unknown-object
      deny-by-default tests.

- [ ] **9. Publish durable commands and events**
  - [ ] **Define versioned NATS subjects and payloads**
    - [ ] Add durable subjects for build requested/completed/failed, release
      published, instance created/revised/paused, attachment changed, update
      requested/completed/rejected/uncertain, and exact run start.
    - [ ] Include stable IDs, schema version, idempotency key, trace/request
      context, and exact provenance in every command.
    - [ ] Avoid embedding parameter secret values, artifact content, or
      unbounded logs in NATS messages.
  - [ ] **Use transactional outboxes and inboxes**
    - [ ] Write commands/events in the same transaction as their authoritative
      state transition.
    - [ ] Deduplicate every consumer by stable command or aggregate transition
      ID.
    - [ ] Make publication retry-safe and observable without causing duplicate
      builds, updates, activations, or runs.
  - [ ] **Add messaging tests**
    - [ ] Test database rollback emits no message.
    - [ ] Test publisher retry and duplicate JetStream delivery.
    - [ ] Test one durable start command for each exact build, update, and
      normal-run idempotency tuple.

- [ ] **10. Make projects first-class in the Phoenix UI**
  - [ ] **Add project navigation**
    - [ ] Change the organization page to list visible projects rather than
      flattening all repositories.
    - [ ] Add `/projects/:project_id` with reusable project tabs for
      Repositories, Agents, Runs, and Settings.
    - [ ] Add project-aware empty, loading, denied, and revoked-access states.
    - [ ] Complete clickable breadcrumbs as Organizations / Organization /
      Project / current resource, leaving only the current page unlinked.
  - [ ] **Add project agent management**
    - [ ] List imported release agents and configured instances separately.
    - [ ] Add authorized import flow with release selection and typed parameter
      form generation.
    - [ ] Show instance lifecycle, active revision/release, state-volume
      health, attachments, update availability, and recent runs.
    - [ ] Add instance detail and revision history views.
    - [ ] Add attachment create/enable/disable/remove controls with repository
      and ref selection.
    - [ ] Add parameter-update and release-update review flows showing old and
      candidate resolved values without displaying secrets.
    - [ ] Distinguish immutable release-owned runtime fields from project
      resource and network selections, and explain that changing release-owned
      fields requires importing a fork as a separate instance.
    - [ ] Show invalid candidate revisions and stable diagnostics for currently
      unsupported state-capability changes and hookless stateful updates.
    - [ ] Stream update-hook status and logs live only after subscription
      reauthorization.
    - [ ] Add recovery controls for uncertain-state paused instances.
  - [ ] **Extend repository navigation**
    - [ ] Add Releases and Agents tabs alongside Files, Commits, and Branches.
    - [ ] Show builds, immutable releases, artifact manifests, exported agents,
      publication state, and provenance on the Releases tab.
    - [ ] Show instances attached to the repository, grouped by project
      instance and ref selector, on the Agents tab.
    - [ ] Link source commits to builds/releases and target runs to their exact
      release and instance revision.
  - [ ] **Use shared UI primitives**
    - [ ] Reuse the design-system Breadcrumbs and Tag components.
    - [ ] Keep project and repository tabs feature-local rather than placing
      domain navigation in the design system.
    - [ ] Add accessible labels, keyboard navigation, focus states, responsive
      layouts, stable DOM IDs, and authorization-safe live refresh.
  - [ ] **Add LiveView and component tests**
    - [ ] Test organization-to-project-to-repository navigation and clickable
      breadcrumbs.
    - [ ] Test RLS-filtered project, release, and instance listings.
    - [ ] Test import, parameter validation, attachment, update, failure, and
      recovery UI flows.
    - [ ] Test live subscription revocation removes access immediately.

- [ ] **11. Add observability, retention, and operator recovery**
  - [ ] **Add structured tracing and metrics**
    - [ ] Trace build, release, instance, revision, attachment, update, run,
      repository, release artifact, actor, and request identifiers.
    - [ ] Measure build/update/run queue time, execution duration, artifact
      sizes, hook outcomes, paused instances, and outbox lag.
    - [ ] Redact secret references and parameter values marked sensitive.
  - [ ] **Define cleanup and retention**
    - [ ] Permanently retain published release, release-agent, agent-family,
      attachment, revision, update, and run metadata needed to resolve
      historical provenance.
    - [ ] Retain artifact bytes while a release is available for new use or any
      instance revision or run references them; garbage-collect bytes only
      after revocation and removal of every retaining reference.
    - [ ] Remove only abandoned build workspaces and unreferenced draft
      artifacts through opaque validated paths.
    - [ ] Preserve instance revisions, update records, logs, and run provenance
      required for audit.
    - [ ] Add reconciliation for orphaned build/update VMs, leases, workspaces,
      and incomplete outbox transitions.
  - [ ] **Add operator tooling**
    - [ ] Add read-only inspection commands for release provenance, active
      instance revision, attachments, leases, updates, and exact run bindings.
    - [ ] Add narrowly scoped recovery commands for paused instances and
      abandoned builds/updates.
    - [ ] Require authorization and privileged audit for every mutating
      recovery action.

- [ ] **12. Verify complete real-system behavior**
  - [ ] **Add a real-PostgreSQL and NATS integration scenario**
    - [ ] Build and publish one reusable reviewer release from a source
      repository.
    - [ ] Import the same release into two projects with different parameters.
    - [ ] Attach each instance to a target repository and push target commits.
    - [ ] Verify exactly one run per matching attachment with exact release,
      instance revision, parameters, target commit, and receive provenance.
    - [ ] Verify independent state volumes and controlled result branches in
      each target repository.
  - [ ] **Add a real-libkrun update scenario**
    - [ ] Publish a second reviewer release containing an update hook.
    - [ ] Create a candidate instance revision and execute the hook with the
      exclusive persistent volume.
    - [ ] Verify exit zero activates the candidate and the next run executes
      the new built binary while retaining state.
    - [ ] Verify explicit nonzero preserves the current revision after the
      agent rolls back its SQLite transaction.
    - [ ] Verify forced termination pauses the instance and prevents normal
      runs until authorized recovery.
    - [ ] Verify update and run cgroups, runtime files, workspaces, and leases
      are cleaned without deleting releases or persistent state.
  - [ ] **Add a Playwright product journey**
    - [ ] Log in through OIDC and navigate Organization → Project → Agents.
    - [ ] Import a published release agent, fill typed parameters, and attach
      it to a repository branch.
    - [ ] Push a target commit and observe the run update without reloading.
    - [ ] Inspect exact agent release, instance revision, target commit, logs,
      artifacts, diff, and proposal.
    - [ ] Publish a newer release, review and start the instance update, watch
      hook output, and verify the active revision changes.
    - [ ] Exercise a failed update and recovery path without exposing
      unauthorized project data.
  - [ ] **Run repository quality gates**
    - [ ] Run `cargo fmt --all -- --check`.
    - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
    - [ ] Run `cargo test --workspace --all-features`.
    - [ ] Run `cargo doc --workspace --all-features --no-deps`.
    - [ ] Run the real-PostgreSQL integration suites.
    - [ ] Run the NATS-backed integration suites.
    - [ ] Run the real libkrun/KVM build, update, and runtime scenarios.
    - [ ] Run `mix precommit` in `web/`.
    - [ ] Run the Playwright browser project.
    - [ ] Run Mélange generated-migration drift detection and
      `melange doctor`.
    - [ ] Run OpenFGA/Mélange compatibility fixtures.
    - [ ] Run `git diff --check`.

- [ ] **13. Document and hand off the completed model**
  - [ ] **Update architecture documentation**
    - [ ] Document release/build provenance and immutable artifact storage.
    - [ ] Document project imports, instance/revision/attachment semantics,
      parameter delivery, and state sharing.
    - [ ] Document release, project, and platform runtime-policy precedence,
      including why a source fork is a distinct agent family and instance.
    - [ ] Document normal-run and update-hook guest mount/protocol contracts.
    - [ ] Document update exit guarantees and the agent's exclusive rollback
      responsibility.
    - [ ] Document authorization/RLS relationships and NATS subjects.
    - [ ] Document project, repository, release, agent, instance, update, and
      run UI navigation.
  - [ ] **Update operator and contributor guidance**
    - [ ] Document local release building, publishing, importing, attaching,
      running, updating, pausing, and recovering.
    - [ ] Document host prerequisites and durable storage locations.
    - [ ] Add explicit warnings that host rollback is not provided and abnormal
      update termination pauses the instance.
    - [ ] Update the root README crate/component map and manual smoke workflow.
    - [ ] Move superseded root `TODO.md` items into this task system or link
      them from appropriately scoped task files.
  - [ ] **Record completion evidence**
    - [ ] Record migration versions, Mélange CLI version, generated-SQL hash,
      release/build fixture IDs, test counts, and real-libkrun evidence.
    - [ ] Record any deliberately deferred work as new files in `tasks/todo/`.
    - [ ] Confirm every required checkbox in this task is complete before
      moving it to `tasks/done/`.

## Completion evidence

Populate this section while the task is in `tasks/in-progress/`. Include exact
commands and results, generated migration provenance, real PostgreSQL/NATS
service versions, libkrun host/runtime versions, the golden build/release/run
IDs, Playwright results, and links to any follow-up task files.
