# Reusable agent releases, project instances, and exact runs

Owner: Codex

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

- [x] **1. Finalize terminology and provider-neutral domain contracts**
  - [x] **Define stable identifiers and value objects**
    - [x] Add provider-neutral `ReleaseId`, `ReleaseArtifactId`,
      `AgentFamilyId`, `ReleaseAgentId`, `BuildRequestId`, `AgentInstanceId`,
      `AgentInstanceRevisionId`, `AgentAttachmentId`, and `AgentUpdateId`
      types.
    - [x] Keep `RunId`, `RepositoryId`, `ProjectId`, commit IDs, refs, and
      configuration hashes as exact typed values at service boundaries.
    - [x] Validate opaque IDs, release versions, agent keys, parameter names,
      artifact paths, and ref selectors without accepting filesystem-derived
      identity.
  - [x] **Define aggregate contracts**
    - [x] Define immutable release, release-artifact, and exported-agent
      domain models.
    - [x] Define the source-repository-owned agent family that identifies one
      exported agent across releases independently of display names and keys
      from other repositories.
    - [x] Define project-owned agent-instance and immutable
      agent-instance-revision models.
    - [x] Define repository/ref attachment and trigger-policy models.
    - [x] Define build-request, instance-update, and exact run-provenance
      models.
    - [x] Document that `VmInstance` remains ephemeral and is never the
      product-level `AgentInstance`.
  - [x] **Add domain tests**
    - [x] Test identifier parsing, serialization, malformed input rejection,
      and path/ref validation.
    - [x] Test valid and invalid lifecycle transitions for releases,
      instances, instance revisions, attachments, builds, and updates.
    - [x] Test that update compatibility requires the same agent family and
      cannot be established by reusing an exported key in another repository.
    - [x] Test deterministic hashes and idempotency keys for builds, releases,
      instance revisions, attachments, updates, and runs.

- [x] **2. Replace the PostgreSQL agent and release data model**
  - [x] **Add release and build records**
    - [x] Add source-repository-owned `agent_families` with stable IDs and
      repository-scoped exported keys.
    - [x] Add repositories' immutable `releases` records with source commit,
      source ref provenance, build-definition hash, build-run ID, manifest
      hash, lifecycle state, publication actor, and timestamps.
    - [x] Add `release_artifacts` with normalized relative paths, kinds, modes,
      content hashes, sizes, media types, storage keys, and provenance.
    - [x] Add `release_agents` linking an agent family, stable exported key,
      and runtime configuration to a release.
    - [x] Add durable build requests, build runs or run-kind metadata, build
      diagnostics, and idempotent build-command/outbox records.
    - [x] Enforce immutable published-release fields and repository-scoped
      uniqueness for release names or versions.
  - [x] **Replace hollow agents with project instances**
    - [x] Replace or rewrite `agents` as project-owned `agent_instances` with
      stable name, lifecycle state, active revision, state-volume ownership,
      and timestamps.
    - [x] Add immutable `agent_instance_revisions` resolving release agent,
      parameter document, parameter hash, secret references, resource policy,
      project network restriction, fully resolved effective runtime policy,
      platform-policy version, creator, and timestamps.
    - [x] Add `agent_attachments` connecting an instance to a repository in
      the same project, with ref selector, trigger policy, enabled state, and
      removal tombstone and audit timestamps.
    - [x] Repoint state volumes, runs, run requests, review provenance, and
      audit records to the instance and exact instance revision.
    - [x] Add database constraints preventing cross-project repository
      attachments and cross-instance revision activation.
  - [x] **Add update state**
    - [x] Add durable agent-update records containing current and candidate
      revisions/releases, stable update ID, actor, state, hook run, exit
      result, timestamps, diagnostics, and final decision.
    - [x] Add lifecycle states for updating, update rejection, unknown-state
      pause, activation-recovery pause, recovery, and successful activation.
    - [x] Add uniqueness and compare-and-swap constraints preventing concurrent
      updates or activation from a stale current revision.
    - [x] Add a transactional instance run gate, durable deferred triggers, and
      constraints preventing migration from starting while a normal request,
      run, or instance-volume lease remains active.
  - [x] **Add indexes and triggers**
    - [x] Index releases by repository and publication state.
    - [x] Index exported agents by release and stable agent key.
    - [x] Index instances by project and lifecycle state.
    - [x] Index attachments by target repository/ref and enabled state.
    - [x] Index instance revisions and updates by instance and creation time.
    - [x] Update UI wakeup triggers for projects, releases, instances,
      attachments, updates, and runs.
  - [x] **Add real-PostgreSQL persistence tests**
    - [x] Test release immutability and artifact uniqueness.
    - [x] Test cross-project imports with authorized release access.
    - [x] Test same release imported into separate projects produces isolated
      instances and volumes.
    - [x] Test attachment project-boundary constraints.
    - [x] Test active-revision CAS and concurrent-update rejection.
    - [x] Test attachment and release tombstones preserve every historical
      foreign-key target.
    - [x] Test run-gate closure, draining, and deferred-trigger creation under
      concurrent receives and dispatch.
    - [x] Test immediate transaction visibility for instance, attachment, and
      update state changes.

- [x] **3. Extend the source configuration and typed parameter model**
  - [x] **Version the configuration schema**
    - [x] Add build command, arguments, working directory, build root image,
      resource limits, network profile, declared artifact outputs, and build
      triggers.
    - [x] Define release runtime command paths relative to the immutable
      release mount rather than arbitrary host paths.
    - [x] Bind the exact runtime executable, arguments, working directory,
      pinned root-image digest, mount contract, state requirement, network
      ceiling, and resource bounds into the immutable release agent.
    - [x] Define project-selectable resource sizing within release bounds and
      networking as a restriction of the release ceiling.
    - [x] Resolve and persist the exact effective runtime policy and
      platform-policy version when validating an instance revision.
    - [x] Reject consumer attempts to replace release-owned runtime fields,
      broaden network access, exceed resource bounds, or use a root image the
      platform does not allow; never substitute another image silently.
    - [x] Define a stable exported agent key separately from its mutable
      display name.
    - [x] Add optional candidate-release update-hook command, shell contract,
      arguments, timeout, and resource limits.
    - [x] Persist whether the exported agent requires instance state as part of
      its release-agent contract.
    - [x] Preserve strict version rejection and structured diagnostics for
      unsupported configuration versions.
  - [x] **Define typed parameters**
    - [x] Support bounded string, integer, boolean, and enum parameter schemas
      with required/default metadata.
    - [x] Reject duplicate, malformed, unbounded, or reserved parameter names.
    - [x] Validate instance parameter values completely before creating an
      instance revision.
    - [x] Canonicalize validated parameter JSON and persist a deterministic
      parameter hash.
    - [x] Keep host-generated run context outside user-configurable
      parameters.
    - [x] Represent secrets only as typed references and reject a runnable
      revision when required bindings cannot be resolved safely.
  - [x] **Add parser and compatibility tests**
    - [x] Test configuration versioning, build definitions, artifact paths,
      runtime commands, parameter types/defaults, and update-hook validation.
    - [x] Test deterministic effective-policy resolution across release
      requirements, project selections, and platform restrictions.
    - [x] Test that project configuration cannot replace the runtime command,
      arguments, working directory, root image, mounts, or state requirement,
      and cannot broaden networking.
    - [x] Test new-release parameter compatibility with existing instance
      values.
    - [x] Produce stable structured diagnostics for a release update that
      changes state capability or omits the hook required by a stateful
      instance.
    - [x] Test diagnostic stability for removed, renamed, newly required, and
      type-changed parameters.
    - [x] Test deterministic normalized configuration and parameter hashes.

- [x] **4. Implement isolated builds and immutable release artifacts**
  - [x] **Create the build workflow**
    - [x] Convert an accepted source push or authorized manual command into an
      idempotent build request for an exact repository, commit, ref, and
      build-definition hash.
    - [x] Materialize the exact source tree read-only and a separate writable
      build workspace.
    - [x] Run the build command in a resource-limited microVM using the pinned
      build root image and declared network policy.
    - [x] Prevent the build guest from receiving canonical Git credentials,
      release-store write access, instance state volumes, or host paths.
    - [x] Capture build logs, metrics, exit results, diagnostics, and declared
      artifact outputs durably.
  - [x] **Seal and import build outputs**
    - [x] Treat build completion as one-way finalization, stop/reap the VM,
      atomically seal the build workspace, and import only from the sealed
      path.
    - [x] Reuse or extend the safe importer to reject symlinks, devices, FIFOs,
      sockets, escapes, unsupported modes, quota violations, and undeclared
      outputs.
    - [x] Construct a deterministic release artifact manifest with hashes,
      sizes, paths, modes, media types, and complete build provenance.
    - [x] Store artifacts under an opaque-ID/content-addressed canonical
      layout that cannot be influenced by repository paths.
    - [x] Ensure retries reuse durable build identity and never create
      conflicting release artifacts.
  - [x] **Publish releases transactionally**
    - [x] Create a draft release only after the build result and complete
      artifact manifest are durable.
    - [x] Add an authorized explicit publish command that freezes the release
      and emits its outbox event.
    - [x] Prevent failed, incomplete, rejected, or already published releases
      from being mutated or republished inconsistently.
    - [x] Retain source commit, build run, normalized configuration, exported
      agent, and artifact hashes as immutable provenance.
  - [x] **Add build and release tests**
    - [x] Test exact-commit builds for two branches with different contents.
    - [x] Test artifact import rejection for every unsafe filesystem type and
      quota boundary.
    - [x] Test build-request and release-publication idempotency.
    - [x] Test published release immutability.
    - [x] Test a built executable runs from the imported read-only release
      tree rather than the source tree.

- [x] **5. Implement project imports, instances, revisions, and attachments**
  - [x] **Import a released agent**
    - [x] Add an authorized command that selects an accessible published
      release agent and creates a project-owned instance.
    - [x] Validate initial parameters and secret references against the
      release-agent schema.
    - [x] Create the initial immutable instance revision and activate it
      atomically with instance creation.
    - [x] Allocate one persistent state volume per instance only when its
      released agent requests state.
    - [x] Make repeated import commands idempotent without conflating imports
      in different projects.
  - [x] **Manage instance revisions**
    - [x] Add authorized parameter, secret-binding, in-bounds resource-sizing,
      and network-restriction update commands that create new immutable
      revisions.
    - [x] Resolve and persist the effective policy from immutable release-owned
      runtime fields, project selections, and current platform constraints.
    - [x] Require a forked or copied implementation to publish its own agent
      family and be imported as a separate instance rather than updating an
      instance from the original family.
    - [x] Never rewrite revisions referenced by an update or run.
    - [x] Validate a candidate revision completely before it may become active.
    - [x] Persist unsupported state-capability changes and hookless stateful
      release updates as visible invalid candidates without starting a guest.
    - [x] Record activation and rejection as durable instance events.
  - [x] **Attach instances to target repositories**
    - [x] Add authorized create, enable, disable, update, and remove attachment
      commands.
    - [x] Implement removal as a tombstone so historical run provenance keeps
      resolving the exact attachment.
    - [x] Validate that the target repository belongs to the instance project.
    - [x] Compile and validate exact refs or bounded ref-prefix selectors.
    - [x] Keep trigger policy on the attachment rather than the imported
      agent's source branch.
    - [x] Permit one instance to share its state across several attachments
      while making separate instances the explicit isolation mechanism.
  - [x] **Add service tests**
    - [x] Test importing one release into several projects with different
      parameters and independent state.
    - [x] Test attaching one instance to several repositories in its project.
    - [x] Test cross-project attachment denial.
    - [x] Test parameter revision history and active-revision immutability.
    - [x] Test allowed resource and network restrictions and reject every
      attempted release-owned runtime override.
    - [x] Test that a release from a forked repository cannot update an
      instance of the original repository's family even when its key matches.
    - [x] Test disabled and removed attachments cannot trigger new runs.
    - [x] Test invalid update candidates expose stable state-compatibility
      diagnostics and leave the active revision and volume untouched.

- [x] **6. Bind normal runs to releases, revisions, attachments, and targets**
  - [x] **Replace direct source-config run creation**
    - [x] Stop treating a target repository's `agent.toml` as an implicit
      project instance that directly runs on push.
    - [x] On an accepted target push, resolve enabled attachments whose ref
      selector matches each accepted update.
    - [x] When an instance run gate is closed, record each matching receive or
      command as one durable deferred trigger without selecting an instance
      revision.
    - [x] Create one idempotent run request per exact attachment, active
      instance revision, target repository, target ref, target commit, and
      receive ID.
    - [x] Recheck instance lifecycle, active revision, attachment state,
      authorization, and release availability in the same transaction that
      creates the durable run request.
    - [x] On reopening the gate, re-evaluate and materialize deferred triggers
      idempotently against the then-active revision, or retain a durable denial
      diagnostic.
  - [x] **Construct the runtime filesystem contract**
    - [x] Mount the exact release artifact tree read-only at `/release`.
    - [x] Mount the exact target source tree read-only at `/workspace/repo`.
    - [x] Provide a separate writable result workspace at `/workspace/work`.
    - [x] Mount the instance's persistent state volume exclusively at
      `/var/lib/hephaestus` when requested.
    - [x] Write canonical validated ordinary parameters to a read-only
      `/run/hephaestus/parameters.json`.
    - [x] Provide host-generated context separately with immutable instance,
      revision, release, attachment, repository, ref, commit, run, and mount
      identifiers.
    - [x] Execute only the released runtime command under the existing
      unprivileged guest identity and host-constrained policy.
    - [x] Enforce the revision's immutable resolved resource and network policy,
      including the release ceiling, project restriction, and platform policy
      recorded when the revision was validated.
    - [x] Revalidate that the immutable resolved policy remains allowed before
      launch; if platform policy has tightened, deny the guest instead of
      silently running it under a different effective policy.
    - [x] Reauthorize release use before every logical artifact acquisition and
      VM start even when the exact artifacts already exist in a host cache.
    - [x] Allow an already materialized and started guest to finish if source
      access is later revoked; never reuse its runnable filesystem for another
      run after that guest is destroyed.
  - [x] **Preserve controlled result publication**
    - [x] Bind result provenance to the exact instance revision and release
      artifact manifest in addition to the target commit.
    - [x] Publish result commits only into the target repository through the
      host-side result publisher.
    - [x] Keep exact target input commit as result parent and preserve CAS
      approval behavior.
  - [x] **Add run tests**
    - [x] Test that identical target commits run differently under two
      instance revisions with different releases or parameters.
    - [x] Test that historical runs retain their exact release and parameter
      provenance after an instance update.
    - [x] Test shared state across attachments of one instance and isolation
      between two instances.
    - [x] Test release, source, work, runtime-parameter, and state mount
      permissions inside a real libkrun guest.
    - [x] Test duplicate receives and NATS redelivery never launch duplicate
      runs.
    - [x] Test deferred triggers bind only the revision active after the gate
      reopens and cannot race an update hook.
    - [x] Test source-access revocation blocks the next guest start despite a
      warm host artifact cache while an already started guest may finish.
    - [x] Test a later platform-policy change either still permits the exact
      resolved revision or denies launch without silently changing execution.

- [x] **7. Implement safe agent-instance updates**
  - [x] **Create candidate updates**
    - [x] Add an authorized update command selecting a published release agent
      from the instance's exact agent family.
    - [x] Resolve and validate candidate parameters, secret references,
      in-bounds resource sizing, network restriction, and the release-owned
      runtime contract into a candidate instance revision.
    - [x] Persist a stable update ID and compare-and-swap expectation for the
      currently active revision.
    - [x] Reject concurrent updates, stale candidates, revoked releases, and
      updates for paused/recovering instances unless explicitly allowed.
    - [x] Reject state-capability changes and hookless release updates for
      stateful instances as visible unsupported candidates before closing the
      run gate or acquiring the volume.
  - [x] **Run the update hook in isolation**
    - [x] In the update-creation transaction, CAS-close the instance run gate
      and begin durably deferring new matching triggers.
    - [x] Drain every pre-gate normal request and run, then prove no normal
      request, run, or lease remains before entering migration.
    - [x] Block all normal dispatch and acquire the instance state volume under
      an exclusive fenced lease.
    - [x] Boot a special update run using the candidate release and its
      constrained update-hook shell inside the guest.
    - [x] Mount candidate and previous releases read-only and expose old/new
      validated parameter documents.
    - [x] Pass stable update, current/candidate revision, and
      current/candidate release identifiers as host-generated context.
    - [x] Enforce update-specific resource, body/output, and wall-clock limits.
    - [x] Persist hook logs, metrics, lifecycle events, exit status, and
      diagnostics without publishing a repository result branch.
  - [x] **Apply the exit contract**
    - [x] On exit zero, durably mark the hook successful and CAS-activate the
      candidate revision before re-enabling normal runs.
    - [x] Treat durable hook success as the irreversible commit point:
      subsequent candidate revocation or authorization change cannot return
      the old revision to service, and activation failure pauses the instance
      for activation recovery.
    - [x] On explicit nonzero exit, record agent-declared rollback completion,
      reject the candidate, retain the current revision, and restore the
      instance's previous runnable lifecycle state.
    - [x] On timeout, signal, VM failure, lost protocol, or host uncertainty,
      retain the current revision but pause the instance as state compatibility
      unknown.
    - [x] Never claim that Hephaestus rolled back agent-owned state.
    - [x] Release the fenced volume lease and clean the update VM/runtime
      resources on every terminal path.
    - [x] Reopen the run gate and materialize deferred triggers only after
      successful activation or an explicit nonzero result that safely restores
      the current revision; keep the gate closed on every uncertain or
      activation-recovery path.
  - [x] **Reconcile crashes and retries**
    - [x] Deliver a stable `HEPHAESTUS_UPDATE_ID` so agent hooks can implement
      idempotency.
    - [x] Reconcile a host crash before hook start without creating a second
      logical update.
    - [x] Reconcile a crash during the hook to the unknown-state paused path.
    - [x] Reconcile a crash after durable hook success but before activation
      by activating that exact candidate without rerunning the migration.
    - [x] Add explicit authorized recovery, retry, reject, and resume commands
      with durable audit events.
  - [x] **Add update-hook tests**
    - [x] Test successful migration activates the candidate release and
      preserves instance identity, attachments, and volume.
    - [x] Test explicit nonzero exit keeps the current revision active.
    - [x] Test timeout, signal, and VM loss pause the instance.
    - [x] Test stale activation and concurrent update rejection.
    - [x] Test that a CAS or invariant anomaly after durable hook success enters
      activation recovery and never runs the previous revision.
    - [x] Test that source-access or release revocation after durable hook
      success cannot veto candidate activation but blocks its next guest start.
    - [x] Test normal requests drain before migration, triggers received behind
      the closed gate are deferred, and no old-revision run starts afterward.
    - [x] Test stable update IDs and crash reconciliation at each durable
      boundary.
    - [x] Test an agent-owned transactional SQLite rollback followed by a
      successful normal run on the current release.

- [x] **8. Extend authorization, RLS, and privileged audit**
  - [x] **Update the canonical OpenFGA model**
    - [x] Add build, release, release-agent use/import, instance management,
      attachment management, update, execute, and recovery permissions with
      relationship inheritance.
    - [x] Keep the agent instance parented by its consuming project.
    - [x] Make release visibility and import permission inherit from the source
      repository/project without copying grants into consuming projects.
    - [x] Define release-use authorization for an installed instance so source
      access revocation blocks every subsequent logical artifact acquisition
      and guest start, independently of host caching.
    - [x] Define target-repository permission requirements for attachment and
      result publication.
  - [x] **Update Mélange inputs and generated SQL**
    - [x] Extend `melange_tuples` from authoritative release, project,
      instance, and attachment relationships.
    - [x] Run the repository-pinned Mélange CLI against the canonical `.fga`
      model.
    - [x] Commit the actual generated specialized permission SQL as a reviewed
      versioned migration.
    - [x] Update drift detection and `melange doctor` coverage.
  - [x] **Apply PostgreSQL RLS**
    - [x] Add `USING` and `WITH CHECK` policies for builds, releases,
      artifacts, release agents, instances, revisions, attachments, updates,
      and their audit metadata.
    - [x] Ensure organization and project list queries filter imported agents
      and instances through RLS without manual tenant predicates.
    - [x] Keep the application role unable to own protected tables or bypass
      RLS and force RLS where appropriate.
  - [x] **Authorize every external side effect**
    - [x] Check permission before build execution, release publication,
      artifact access, agent import, attachment changes, normal execution,
      update-hook execution, recovery, and result publication.
    - [x] Recheck release use immediately before materializing each new normal
      or update guest; do not terminate a guest whose authorized artifact
      acquisition and start already completed.
    - [x] Reauthorize live subscriptions before delivering release, instance,
      update, or run events.
    - [x] Write structured privileged audit events with actor, permission,
      object, decision, request ID, and authorization-model version.
  - [x] **Add authorization tests**
    - [x] Test release import across authorized and unauthorized projects.
    - [x] Test revoking source access leaves instance and run history visible
      but blocks new normal and update guests even with cached artifacts.
    - [x] Test instance management, attachment, execution, update, and recovery
      decisions for owners, maintainers, members, and outsiders.
    - [x] Test RLS list/read/insert/update/delete behavior and normal-role
      non-bypass.
    - [x] Update OpenFGA/Mélange parity fixtures and unknown-object
      deny-by-default tests.

- [x] **9. Publish durable commands and events**
  - [x] **Define versioned NATS subjects and payloads**
    - [x] Add durable subjects for build requested/completed/failed, release
      published, instance created/revised/paused, attachment changed, update
      requested/completed/rejected/uncertain, and exact run start.
    - [x] Include stable IDs, schema version, idempotency key, trace/request
      context, and exact provenance in every command.
    - [x] Avoid embedding parameter secret values, artifact content, or
      unbounded logs in NATS messages.
  - [x] **Use transactional outboxes and inboxes**
    - [x] Write commands/events in the same transaction as their authoritative
      state transition.
    - [x] Deduplicate every consumer by stable command or aggregate transition
      ID.
    - [x] Make publication retry-safe and observable without causing duplicate
      builds, updates, activations, or runs.
  - [x] **Add messaging tests**
    - [x] Test database rollback emits no message.
    - [x] Test publisher retry and duplicate JetStream delivery.
    - [x] Test one durable start command for each exact build, update, and
      normal-run idempotency tuple.

- [x] **10. Make projects first-class in the Phoenix UI**
  - [x] **Add project navigation**
    - [x] Change the organization page to list visible projects rather than
      flattening all repositories.
    - [x] Add `/projects/:project_id` with reusable project tabs for
      Repositories, Agents, Runs, and Settings.
    - [x] Add project-aware empty, loading, denied, and revoked-access states.
    - [x] Complete clickable breadcrumbs as Organizations / Organization /
      Project / current resource, leaving only the current page unlinked.
  - [x] **Add project agent management**
    - [x] List imported release agents and configured instances separately.
    - [x] Add authorized import flow with release selection and typed parameter
      form generation.
    - [x] Show instance lifecycle, active revision/release, state-volume
      health, attachments, update availability, and recent runs.
    - [x] Add instance detail and revision history views.
    - [x] Add attachment create/enable/disable/remove controls with repository
      and ref selection.
    - [x] Add parameter-update and release-update review flows showing old and
      candidate resolved values without displaying secrets.
    - [x] Distinguish immutable release-owned runtime fields from project
      resource and network selections, and explain that changing release-owned
      fields requires importing a fork as a separate instance.
    - [x] Show invalid candidate revisions and stable diagnostics for currently
      unsupported state-capability changes and hookless stateful updates.
    - [x] Stream update-hook status and logs live only after subscription
      reauthorization.
    - [x] Add recovery controls for uncertain-state paused instances.
  - [x] **Extend repository navigation**
    - [x] Add Releases and Agents tabs alongside Files, Commits, and Branches.
    - [x] Show builds, immutable releases, artifact manifests, exported agents,
      publication state, and provenance on the Releases tab.
    - [x] Show instances attached to the repository, grouped by project
      instance and ref selector, on the Agents tab.
    - [x] Link source commits to builds/releases and target runs to their exact
      release and instance revision.
  - [x] **Use shared UI primitives**
    - [x] Reuse the design-system Breadcrumbs and Tag components.
    - [x] Keep project and repository tabs feature-local rather than placing
      domain navigation in the design system.
    - [x] Add accessible labels, keyboard navigation, focus states, responsive
      layouts, stable DOM IDs, and authorization-safe live refresh.
  - [x] **Add LiveView and component tests**
    - [x] Test organization-to-project-to-repository navigation and clickable
      breadcrumbs.
    - [x] Test RLS-filtered project, release, and instance listings.
    - [x] Test import, parameter validation, attachment, update, failure, and
      recovery UI flows.
    - [x] Test live subscription revocation removes access immediately.

- [x] **11. Add observability, retention, and operator recovery**
  - [x] **Add structured tracing and metrics**
    - [x] Trace build, release, instance, revision, attachment, update, run,
      repository, release artifact, actor, and request identifiers.
    - [x] Measure build/update/run queue time, execution duration, artifact
      sizes, hook outcomes, paused instances, and outbox lag.
    - [x] Redact secret references and parameter values marked sensitive.
  - [x] **Define cleanup and retention**
    - [x] Permanently retain published release, release-agent, agent-family,
      attachment, revision, update, and run metadata needed to resolve
      historical provenance.
    - [x] Retain artifact bytes while a release is available for new use or any
      instance revision or run references them; garbage-collect bytes only
      after revocation and removal of every retaining reference.
    - [x] Remove only abandoned build workspaces and unreferenced draft
      artifacts through opaque validated paths.
    - [x] Preserve instance revisions, update records, logs, and run provenance
      required for audit.
    - [x] Add reconciliation for orphaned build/update VMs, leases, workspaces,
      and incomplete outbox transitions.
  - [x] **Add operator tooling**
    - [x] Add read-only inspection commands for release provenance, active
      instance revision, attachments, leases, updates, and exact run bindings.
    - [x] Add narrowly scoped recovery commands for paused instances and
      abandoned builds/updates.
    - [x] Require authorization and privileged audit for every mutating
      recovery action.

- [x] **12. Verify complete real-system behavior**
  - [x] **Add a real-PostgreSQL and NATS integration scenario**
    - [x] Build and publish one reusable reviewer release from a source
      repository.
    - [x] Import the same release into two projects with different parameters.
    - [x] Attach each instance to a target repository and push target commits.
    - [x] Verify exactly one run per matching attachment with exact release,
      instance revision, parameters, target commit, and receive provenance.
    - [x] Verify independent state volumes and controlled result branches in
      each target repository.
  - [x] **Add a real-libkrun update scenario**
    - [x] Publish a second reviewer release containing an update hook.
    - [x] Create a candidate instance revision and execute the hook with the
      exclusive persistent volume.
    - [x] Verify exit zero activates the candidate and the next run executes
      the new built binary while retaining state.
    - [x] Verify explicit nonzero preserves the current revision after the
      agent rolls back its SQLite transaction.
    - [x] Verify forced termination pauses the instance and prevents normal
      runs until authorized recovery.
    - [x] Verify update and run cgroups, runtime files, workspaces, and leases
      are cleaned without deleting releases or persistent state.
  - [x] **Add a Playwright product journey**
    - [x] Log in through OIDC and navigate Organization → Project → Agents.
    - [x] Import a published release agent, fill typed parameters, and attach
      it to a repository branch.
    - [x] Push a target commit and observe the run update without reloading.
    - [x] Inspect exact agent release, instance revision, target commit, logs,
      artifacts, diff, and proposal.
    - [x] Publish a newer release, review and start the instance update, watch
      hook output, and verify the active revision changes.
    - [x] Exercise a failed update and recovery path without exposing
      unauthorized project data.
  - [x] **Run repository quality gates**
    - [x] Run `cargo fmt --all -- --check`.
    - [x] Run `cargo clippy --workspace --all-targets --all-features`.
    - [x] Run `cargo test --workspace --all-features`.
    - [x] Run `cargo doc --workspace --all-features --no-deps`.
    - [x] Run the real-PostgreSQL integration suites.
    - [x] Run the NATS-backed integration suites.
    - [x] Run the real libkrun/KVM build, update, and runtime scenarios.
    - [x] Run `mix precommit` in `web/`.
    - [x] Run the Playwright browser project.
    - [x] Run Mélange generated-migration drift detection and
      `melange doctor`.
    - [x] Run OpenFGA/Mélange compatibility fixtures.
    - [x] Run `git diff --check`.

- [x] **13. Document and hand off the completed model**
  - [x] **Update architecture documentation**
    - [x] Document release/build provenance and immutable artifact storage.
    - [x] Document project imports, instance/revision/attachment semantics,
      parameter delivery, and state sharing.
    - [x] Document release, project, and platform runtime-policy precedence,
      including why a source fork is a distinct agent family and instance.
    - [x] Document normal-run and update-hook guest mount/protocol contracts.
    - [x] Document update exit guarantees and the agent's exclusive rollback
      responsibility.
    - [x] Document authorization/RLS relationships and NATS subjects.
    - [x] Document project, repository, release, agent, instance, update, and
      run UI navigation.
  - [x] **Update operator and contributor guidance**
    - [x] Document local release building, publishing, importing, attaching,
      running, updating, pausing, and recovering.
    - [x] Document host prerequisites and durable storage locations.
    - [x] Add explicit warnings that host rollback is not provided and abnormal
      update termination pauses the instance.
    - [x] Update the root README crate/component map and manual smoke workflow.
    - [x] Move superseded root `TODO.md` items into this task system or link
      them from appropriately scoped task files.
  - [x] **Record completion evidence**
    - [x] Record migration versions, Mélange CLI version, generated-SQL hash,
      release/build fixture IDs, test counts, and real-libkrun evidence.
    - [x] Record any deliberately deferred work as new files in `tasks/todo/`.
    - [x] Confirm every required checkbox in this task is complete before
      moving it to `tasks/done/`.

## Completion evidence

Populate this section while the task is in `tasks/in-progress/`. Include exact
commands and results, generated migration provenance, real PostgreSQL/NATS
service versions, libkrun host/runtime versions, the golden build/release/run
IDs, Playwright results, and links to any follow-up task files.

Evidence recorded 2026-07-29:

- `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets --all-features`,
  `cargo test --workspace --all-features`, and
  `cargo doc --workspace --all-features --no-deps` all passed after the
  compatibility-breaking exact-instance conversion.
- `cargo test -p release-domain`: 7 provider-neutral identifier, value,
  lifecycle, family-boundary, policy, typed-parameter, hash, and idempotency
  tests passed.
- `cargo test -p agent-config`: 6 versioned parser tests passed, including
  reusable build/runtime declarations, typed parameters, symbolic secret slots,
  normalized hashing, unsafe paths, and rejection of tenant secret material.
- `cargo test -p release-service --test postgres -- --nocapture` against
  PostgreSQL 17.10 passed: draft completion/idempotency, explicit publication,
  published immutability, the same release imported into two projects with
  different parameters and isolated state-volume identities, required-slot
  unrunnable diagnostics, exact attachments, and cross-project denial.
- `scripts/check-openfga-model.sh`: 6 tests and 79 checks passed.
- `scripts/check-authz.sh` with Mélange 0.8.5: schema checksum
  `76e7043ed8a534103adff658f24be57485646163ab73a8e33c2dc6d56c91d298`;
  665 generated functions; 12 doctor checks passed with zero warnings/errors.
- `HEPHAESTUS_POSTGRES_TEST_URL=... cargo test -p authz-postgres --test
  postgres --all-features -- --nocapture` passed against PostgreSQL 17.10.
  The fixture evaluated 80 generated Mélange decisions across builds,
  releases, release agents, instances, attachments, updates, runs, and state
  volumes; it also verified normal-role list/read/insert/update/delete denial
  and unknown permission/object fail-closed behavior.
- The authorization fixture now separates source and consuming projects.
  Removing only the source-project maintainer relation denies live
  `release_agent.can_use` while preserving target attachment execution,
  instance update authority, and historical instance visibility. Normal and
  update launch both require that live source relation, independently of
  already materialized artifact storage.
- `cargo test -p run-orchestrator --test postgres
  commands_transitions_and_outbox_are_idempotent`: exact instance, revision,
  release, release-agent, and attachment provenance persisted idempotently.
- `cargo test -p volume-local --test postgres`: instance-owned state
  allocation, exclusive fenced leases, conflict, detach, and supervised stale
  lease recovery passed against PostgreSQL 17.10.
- `cargo test -p forge-service --test postgres
  persists_exact_config_and_deduplicates_receive`: duplicate receives created
  one attachment-driven exact request, and an invalid target `agent.toml`
  could not suppress the attached instance trigger.
- Migrations `0005_releases_instances_and_secrets.sql`,
  `0006_melange_releases_and_secrets.sql`, and
  `0007_release_secret_rls.sql` plus the compatibility-breaking
  `0008_remove_legacy_agents.sql` applied cleanly to PostgreSQL 17.10; the
  hollow `agents`, `agent_state_volumes`, and `volume_leases` tables and all
  legacy run columns are absent.
- `HEPHAESTUS_APP_LIBKRUN_E2E=1 scripts/run-libkrun-integration.sh` passed
  against libkrun 1.19.0 and libkrunfw 5.5.0 on Fedora 44. Authenticated Git
  push started build `63f6d2f6-bd9e-4787-a07d-607312393406` in a real build
  microVM, imported the declared executable, ran exact normal run
  `8cf4d120-2d74-4d22-9f28-5799228993af` in a second microVM, published its
  controlled result commit, and left no runtime directory or per-VM cgroup.
- The real daemon scenario exposed and fixed the libkrun 36-byte virtio-fs tag
  ceiling for exact release, context, and previous-release runtime mounts.
  `cargo test -p run-runtime-local --all-features` now locks all three
  deterministic tags to exactly 36 bytes.
- `scripts/run-ui-e2e.sh` passed two Chromium journeys through local OIDC,
  authenticated Git smart HTTP, PostgreSQL RLS, JetStream, the deterministic
  guest, and Phoenix LiveView. The tests observed live run/result updates,
  inspected runtime metrics and result diffs, approved and rejected durable
  proposals, and verified the approved Git ref.
- `HEPHAESTUS_POSTGRES_TEST_URL=... HEPHAESTUS_NATS_TEST_URL=... cargo test
  -p build-orchestrator -p release-service --all-features` passed against a
  fresh PostgreSQL 17.10 database and isolated NATS. The suites verify
  transactional build-failed, attachment-changed, instance-paused, and
  update completed/rejected/uncertain events; stable schema/message/idempotency
  fields; one exact update start on command replay; no outbox message after a
  denied transaction; and JetStream acknowledgement-loss deduplication.
  Existing forge and run PostgreSQL suites verify one build and normal-run
  start for their exact idempotency tuples.
- The release-service PostgreSQL fixture now revokes a published update
  release and removes its attachment after a completed normal run, then joins
  that historical run through its exact revision, release agent, revoked
  release, and removed attachment. Replaying either tombstone is idempotent
  and emits one transactional event. The same fixture reads instance,
  attachment, and update state plus their inbox/outbox transitions immediately
  after each service transaction returns.
- The authorization fixture now evaluates 80 Mélange decisions, including
  `release.can_revoke` allow/deny parity. `scripts/check-openfga-model.sh`
  remains 6/6 tests and 79/79 checks; Mélange doctor remains 12/12 with all
  665 generated functions present.
- `HEPHAESTUS_PHASE1B_INTEGRATION=1 scripts/run-libkrun-integration.sh`
  passed against libkrun 1.19.0/libkrunfw 5.5.0 on Fedora 44. Seven real
  microVM runs shared one instance-owned ext4/SQLite volume: two normal runs,
  a competing writer denial, an exit-zero update activated through
  `ReleaseService`, a subsequent candidate-release run retaining state, an
  exit-23 hook whose SQLite transaction rolled back and preserved the active
  revision, and a force-terminated hook reconciled to
  `paused_unknown_state` with its run gate closed. All update/run leases,
  runtime directories, and cgroups were gone; all four releases and the
  persistent state backing remained.
- The release-service fixture revokes the candidate release after the hook's
  durable success commit and before activation recovery, then activates that
  exact candidate without rerunning the hook. The exact-runtime loader and
  live-launch authorization tests independently reject the revoked release
  for the next guest even when its artifact is already cached. Stable update
  IDs are replayed across admission, hook retry, terminal-result replay, and
  post-commit activation recovery boundaries.
- The provider-neutral launch race test revokes authority from inside the
  first VM's provision boundary. That already-started guest completes, while a
  second exact run sharing the warm runtime manager is denied before another
  artifact preparation or provider call. Together with the real Mélange
  source-relation revocation fixture, this proves a cache is never authority.
- The real daemon golden agent now asserts the complete guest filesystem
  contract: `/release`, `/workspace/repo`, and `/run/hephaestus` reject writes;
  the parameters and host context are readable; `/workspace/work` accepts
  controlled result edits; and `/var/lib/hephaestus` accepts persistent state.
  The KVM guest exits zero only if every permission boundary holds.
- Migration `0009_operational_observability.sql` applied cleanly to fresh
  PostgreSQL 17.10 and has SHA-256
  `77d6d9cd5c04f495d1106e42d538087fa9c33914d9f2c0d6d00466b27429cb16`.
  Independent aggregates prevent join multiplication; the application-role
  operator smoke successfully inspected release provenance and RLS-filtered
  release, instance, and secret metrics.
- `scripts/run-ui-e2e.sh` passed 2 Chromium journeys through isolated OIDC,
  Git smart HTTP, PostgreSQL, JetStream, the daemon, and Phoenix. It imported
  v1, attached the exact repository, observed and approved a live normal run,
  activated v2, reconciled an uncertain v3 update by authorized rejection,
  reopened the run gate, and rejected a second durable result. The journey
  exposed and fixed durable request dispatch, post-rejection trigger/launch
  admission, and LiveView connected-mount races.
- Real KVM verification passed on Fedora 44/kernel 6.19.10 with libkrun 1.19.0
  and libkrunfw 5.5.0: daemon build/normal-run golden path, provider smoke,
  and the Phase 1B exit-zero activation, subsequent candidate binary,
  exit-23 rollback, forced-termination pause, retained SQLite state, and
  complete runtime/lease/cgroup cleanup.
- Final authorization evidence: OpenFGA 6/6 tests and 79/79 checks; Mélange
  0.8.5 doctor 12/12; 665 generated functions; tuple-source SHA-256
  `543e99b9827b2f8dc077e510184f0d4c352beb51e568e704574d2ef49f7c6897`;
  generated migration SHA-256
  `698f7b637f16cf7eb592f99308c46e739950a50e673c3aa5bbe9ed7c869eb1a0`.
- Final repository gates passed: 166 Rust workspace tests, 43 serial
  PostgreSQL/NATS package tests, 25 Phoenix tests, strict formatting/Clippy,
  rustdoc, `git diff --check`, three real-KVM modes, the operator smoke, and
  the Playwright product journey.
- The organization workspace now keeps a consistent routed Projects/Secrets
  header across listing and form pages and reuses one presentational resource
  list for projects, owned secrets, and bounded grants. Both final Chromium
  journeys passed with the new routes and action placement.
