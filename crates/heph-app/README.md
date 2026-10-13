# Purpose

`hephaestus-app` is the production composition root and daemon lifecycle for
the Hephaestus distribution. It turns validated configuration into a running
installation with concrete database, NATS, Git, VM, gateway, identity,
secret, image, Forge, and runtime providers.

# Responsibilities

`HephaestusApp` builds the dependency graph, validates startup configuration,
installs RPC routes and background workers, starts supervised listeners and
consumers, and shuts them down in an orderly sequence. The composition layer
converts generated transport values into core contracts, rechecks live
authority at launch boundaries, and starts reconciliation for outboxes, builds,
runs, secrets, and gateway services.

The VM spec factory also validates complete selected volume sets against the
exact persisted run pins and frozen release catalog. It builds only the base
workload; the run orchestrator installs the checked disks and initialization
purposes. This validation grants no launch or volume authority. Canonical launch
still requires a supported live authorizer and authenticated admission, and the
application composition currently uses the existing scalar profile. Mailbox and
unadmitted direct invocations gain no support from this factory method.

The runtime authority manager separately supports explicit
`with_canonical_live_authority` configuration. It validates owned producer and
source authority before preparation, while issuing only through the existing
Provisioning path. Reauthorization reads the persisted session identity,
generation, snapshot hash and validity interval; it creates no credentials or
handoff. Existing pending sessions may become active without changing that
ceiling. Running and normal drain require the original live session; absent,
orphaned, revoked, elapsed or contradictory records deny rather than reissue.
Default composition retains the scalar manager. Canonical caller authorization,
typed mount grants, secret liveness, producer support and app activation remain
separate requirements.

`CanonicalRunLaunchAuthorizer` is an explicit wrapper around the existing scalar
launch adapter. Its scalar method delegates unchanged. Its canonical method,
including calls for empty volume sets, loads the shared persisted live request,
validates its source/session/generic ceiling, and obtains the original DB actor
and request. The control-plane adapter then rechecks that exact correlation and
audits current source use plus attachment execution or instance update rights.
Mutable update actor changes cannot replace the original provision admission
actor. These checks create no runtime credentials or typed volume grants.
Application composition does not select this wrapper yet; mailbox and unadmitted
invocation provenance remain unsupported in its canonical method.

The `hephaestusd` binary is the distribution entry point. Trusted bootstrap
utilities live under `bootstrap/`; they seed operator or end-to-end fixtures
through explicit boundaries. Product meaning remains in `heph-core`, while
host behavior is selected from `heph-std` during composition.

`PreparedInstanceImporter` is a backend step for one frozen recipe instance.
It prepares the complete deployment before claiming, reloads current original
Install authority and the active immutable claim, projects the exact typed import
and calls the real release service. Only an exact successful call is recorded as
creation. Protected completion replays without another import; uncertain calls
keep the original claim for authorized reconciliation. Private113 serializes
prepared births with removal and checks committed input/creation lineage. This
module does not enable dispatch or prove physical volume/VM state. Unknown import
recovery after creator/source withdrawal remains a separate backend boundary.

# When

Use this crate when assembling or running the local Hephaestus distribution.
Call `HephaestusApp::build` with `AppConfig` during startup, then call
`start` to bind listeners and begin supervised work; call `shutdown` when the
process receives its termination signal. The normal binary is
`hephaestusd`.

## One owned volume effect

`recipe_owned_volume::PreparedOwnedVolumeCreator` is an explicit backend service
for one original prepared Install Volume/Create. It uses fresh whole-preparation
and active-claim admission at each Local first-write boundary, preserves the
original provisioning operation across interruption, and retains actual Ready
custody through protected recording and completion even if the caller cancels.
Completed authorized replay does not probe or format. It requires the dedicated
first-birth runtime lease exclusion guard and an already configured disk owner.
It is not composed into daemon startup or a whole recipe engine. Ready-removal
after source withdrawal remains a separate manager-qualified retention step.

## Retaining a ready owned volume

A current project and volume manager can close attachments, let the known run
owners drain their workloads, verify the original disk without writes, and retain
its data. Removing an installation does not borrow the original installer's
identity or require continued access to its old Source.

The service keeps data held when a consumer, lease, owner, original creation, or
physical disk cannot be checked. Actual journal custody stays owned through the
worker fact and manager's commit, including caller cancellation. Retention leaves
attachments closed; a new consumer needs fresh authorization and an explicit
reopening receipt. The current reopening path requires an originally completed
installation. An unfinished but physically ready birth retains its distinct
positive fact and remains held for reuse.

This opt-in backend path does not initialize roots, format disks, rewrite original
installation receipts, reset lease generations, enable named dispatch, or provide
physical VM shutdown evidence from database status. The checked scope pins come
from the configured provider and remain separate from volume-root ownership.


## Installing a prepared recipe

`PreparedRecipeApplication` combines the authenticated planner, durable saved
inputs and existing owning volume and instance adapters. Installation follows
recorded dependency order and reloads progress after each resource. A fresh
request can resume the original command and its original attempts; retries do
not select replacement resources or rewrite parameters. Native disk custody
remains with the owning completion driver when a caller is cancelled.

Installation Ready and runtime activation are separate. Original instance
creation must complete before the qualified activation service may open its
initial gate. A paused or permanently closed gate stays closed on replay; the
installation ledger can still be Installed with activation held. Inspection
returns ledger and saved preparation, and explicitly reports activation as
unsupported until a dedicated authoritative reader is provided. Returned
activation admissions describe the actual call, not later live state.

The engine is injected backend composition only. External verification and
removal/drain stay held for their separately qualified adapters. It does not
start guests, issue Invocations, initialize roots, expose RPC/UI routes or enable
daemon startup. Database and native-disk installation checks are not a complete
SQLite Put/Get or VM lifecycle proof.
