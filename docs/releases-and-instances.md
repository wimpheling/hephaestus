# Reusable releases and project agent instances

Hephaestus separates agent software from a consuming project's installation:

```text
exact source commit
→ isolated build request
→ immutable release and exported release agent
→ project-owned agent instance
→ immutable instance revision
→ repository/ref attachment
→ exact normal or update run
```

Display names do not establish compatibility. An `agent_family` belongs to one
source repository and stable exported key. A copied or forked repository has a
different family even if it publishes the same key.

## Source configuration and builds

Version 2 `agent.toml` owns the build command, pinned build root image,
resources, network profile, and declared outputs. It also owns the released
runtime executable, arguments, working directory, root image, mount contract,
state requirement, resource bounds, network ceiling, parameter schema, secret
slot declarations, and optional update hook.

An accepted matching push writes an idempotent `build_request` and
`hephaestus.build.requested.v1` outbox event for an exact repository, commit,
ref, normalized configuration hash, and build-definition hash. The isolated
build worker:

- reauthorizes `build.can_execute` for the original requester before claiming;
- materializes regular blobs from the exact bare-Git commit into a read-only
  source tree without `.git`, symlinks, or submodules;
- gives the VM only that source tree and a separate empty writable output;
- supplies no Git credentials, secret mount, state volume, environment
  credentials, canonical repository, or release-store access;
- stops and destroys the VM before sealing or importing output;
- rejects undeclared, missing, linked, special, oversized, or escaping output;
- imports each file under a stable opaque operation-derived identity without
  overwriting an existing canonical object.

The `build_executions` record retains bounded logs, metrics, exit information,
the stable release identities, and the complete imported manifest. Crashes
before sealing reap the orphan VM and retry the same build identity. Crashes at
the sealed or imported boundary resume import or release finalization without
launching another VM. A draft release is created only from a durable complete
manifest. Explicit authorized publication freezes it.

## Instances, revisions, and attachments

Importing a published `release_agent` creates one project-owned
`agent_instance`, its initial immutable revision, and—only when requested—one
instance-owned state volume. Separate imports create separate state even when
they select the same release.

A revision contains:

- the exact release agent and family;
- canonical validated ordinary parameters and their hash;
- opaque secret-binding identifiers, never values;
- project-selected resources and networking within release/platform ceilings;
- the fully resolved effective runtime policy and platform-policy version;
- runnable status and stable non-sensitive diagnostics.

Changing parameters, policy restrictions, or secret bindings creates a new
revision. Historical revisions referenced by runs or updates are never
rewritten.

An attachment selects one repository in the instance's project, a validated
exact ref or bounded prefix, and a push/manual trigger policy. Disable and
remove stop new runs; removal is a tombstone so historical provenance remains
resolvable.

## Exact normal runs

A matching receive creates one idempotent request binding the attachment,
then-active instance revision, release and release agent, target repository,
ref, commit, receive, and attempt. The guest sees:

| Guest path | Contract |
| --- | --- |
| `/release` | Fresh read-only tree reconstructed from the exact immutable release manifest |
| `/workspace/repo` | Exact target commit, read-only |
| `/workspace/work` | Separate writable result workspace |
| `/run/hephaestus/parameters.json` | Canonical ordinary parameters, read-only |
| `/run/hephaestus/context.json` | Host-generated exact IDs and provenance, read-only |
| `/var/lib/hephaestus` | Exclusive instance state, only when declared |

Only the host result importer may publish into the target repository. The
result commit keeps the exact target commit as parent and records the exact
release, revision, attachment, and artifact manifest.

## Stateful updates

An update candidate must use the same family and fully validate its parameters,
secret references, policy, and state contract. Stateful release changes
require an update hook. Unsupported state transitions remain visible invalid
candidates and never close the run gate.

A valid update atomically closes the instance gate. Pre-gate requests and runs
drain; new matching triggers become durable deferred records without choosing
a revision. Once no normal run/request/volume lease remains, an update run
acquires the instance volume exclusively and boots the candidate hook in a VM.
Candidate and previous releases and old/new parameter documents are host-owned
read-only inputs. `HEPHAESTUS_UPDATE_ID` is stable.

The exit contract is deliberately asymmetric:

- exit zero is an irreversible hook commit. The candidate must be activated;
  an activation anomaly pauses for activation recovery and never returns the
  old revision to service;
- explicit nonzero means the agent reports that its own rollback completed,
  so the candidate is rejected and the prior revision can run;
- timeout, signal, VM/protocol loss, or host uncertainty leaves compatibility
  unknown and pauses the instance with its run gate closed.

Hephaestus coordinates fencing, cleanup, durable decisions, and activation. It
does not claim to roll back agent-owned state.

Paused updates require an actor with `can_recover` on the exact instance and
an idempotency key. The explicit recovery actions are:

- `RetryHook`, only for `compatibility_unknown` /
  `paused_unknown_state`: release the recovery lease, return the same update
  to draining, and start a later hook attempt with the same
  `HEPHAESTUS_UPDATE_ID`. The hook must use that ID to make retries safe.
- `RejectCandidate`, only for that same uncertain state: keep the prior
  revision and reopen the gate. This records an operator decision; it does not
  assert that Hephaestus restored agent-owned state.
- `ResumeActivation`, only for `activation_recovery` /
  `paused_activation_recovery`: finish the already committed activation
  without re-running the hook. The instance's active revision must still be
  either the expected prior revision or the candidate.

Every recovery action records the actor and request in the command inbox,
appends instance history, and emits a redacted durable recovery event.

## Authorization, RLS, and durable events

Release authority remains rooted in the source repository. Publication and
revocation require the corresponding release relation; importing or launching
an exported agent rechecks `can_use` on the exact `release_agent`. An instance
inherits management, execution, update, and recovery authority from its
consuming project. An attachment additionally requires authority over its exact
target repository.

Protected release, instance, revision, attachment, update, and run tables use
forced RLS. Request transactions set the exact actor and request identities
before calling generated Mélange checks. The launch boundary checks release use
and attachment execution (or instance update) again immediately before
materialization, and records the decision in structured authorization audit
rows. A revoked release remains readable through historical foreign keys but
the VM specification factory rejects it for a new guest.

State changes and their messages share one PostgreSQL transaction. The daemon
publishes pending outbox rows with the outbox UUID as `Nats-Msg-Id`, making
acknowledgement-loss retries deduplicated by JetStream. Current subjects are:

- `hephaestus.build.requested.v1`, `hephaestus.build.completed.v1`, and
  `hephaestus.build.failed.v1`;
- `hephaestus.release.published.v1` and
  `hephaestus.release.revoked.v1`;
- `hephaestus.agent_instance.created.v1` and
  `hephaestus.agent_instance.revised.v1`, plus
  `hephaestus.agent_instance.attachment_changed.v1` and
  `hephaestus.agent_instance.paused.v1`;
- `hephaestus.agent_update.requested.v1`,
  `hephaestus.agent_update.hook_started.v1`,
  `hephaestus.agent_update.hook_committed.v1`,
  `hephaestus.agent_update.completed.v1`,
  `hephaestus.agent_update.rejected.v1`,
  `hephaestus.agent_update.uncertain.v1`, and
  `hephaestus.agent_update.recovered.v1`;
- `hephaestus.instance.run.requested.v1` and the actionable
  `hephaestus.run.start`.

Payloads carry a schema version, stable message/idempotency IDs, nullable
request/trace context, and exact transition provenance. They never carry
artifact bytes, secret values, parameter-secret values, reusable runtime
credentials, or unbounded guest logs. Worker-originated transitions use null
request/trace context rather than inventing a caller.

Revocation is a lifecycle tombstone, not deletion. It immediately prevents
new imports and guest starts while retaining the release, release agent,
artifacts, instance revisions, attachments, and run/result provenance needed
to explain historical execution.

## Browser navigation

The LiveView control plane uses:

```text
Organizations / Organization / Project / current resource
```

Organization pages list visible projects. Project tabs expose repositories,
configured instances, exact runs, and authorization-filtered settings.
Instance detail shows lifecycle, run gate, immutable revisions, attachments,
and update history. Repository tabs add release provenance and attached
project instances alongside files, commits, and branches.

## Local workflow and operator recovery

`scripts/run-local.sh` starts the persistent PostgreSQL, JetStream, OIDC,
Rust-daemon, and Phoenix development stack. The browser flow can publish or
select a release, import its exported agent into a project, set typed
parameters, attach an exact repository ref, push a target commit, inspect the
result, start a reviewed update, and choose an authorized recovery action.
`scripts/run-ui-e2e.sh` performs that workflow from a clean isolated stack.
`scripts/run-libkrun-integration.sh` exercises the real KVM guest contract;
set `HEPHAESTUS_PHASE1B_INTEGRATION=1` with a dedicated PostgreSQL URL for the
stateful update/rollback scenario.

Durable roots are configured independently: repository bare objects,
immutable release artifacts, build workspaces, run workspaces/results,
instance state volumes, runtime trees, and the secret key ring must not
overlap. PostgreSQL retains lifecycle/provenance records and JetStream retains
published command/event messages. The local launcher stores its persistent
development data beneath `.local/hephaestus` and in its named Podman volumes.

Build the authorization-aware operator CLI with
`cargo build -p hephaestus-app --bin hephaestus-operator`. It accepts:

```text
inspect-release <actor> <release> [request]
inspect-instance <actor> <instance> [request]
inspect-secret <actor> <secret> [request]
metrics <actor> [request]
recover-update <actor> <update> <retry|reject|resume> [request]
abandon-build <actor> <build> [request]
```

Every command uses `HEPHAESTUS_DATABASE_URL`. Inspections reauthorize and read
only RLS-filtered metadata; secret inspection uses the metadata-only version
view and cannot query ciphertext. Mutating commands require the exact recovery
permission and write privileged decision audit.

The host never rolls back agent state. An update hook owns its transaction and
must exit nonzero only after completing its own rollback. An abnormal exit or
forced termination yields `paused_unknown_state` with the run gate closed.
Operators must inspect the exact hook run and state before using retry, reject,
or resume; recovery records a decision but cannot prove or reconstruct a
guest-owned rollback.
