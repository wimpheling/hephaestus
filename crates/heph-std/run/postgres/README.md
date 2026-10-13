# Purpose

`run-postgres` persists the durable run state machine behind the core run
orchestrator. It records idempotent start commands, exact release and
attachment provenance, volume and VM bindings, transitions, outcomes,
cancellation, recovery candidates, and ordered VM events.

`PgRunCleanupRepository` persists an exact provider owner namespace, host and VM
identity before provisioning IO. Cleanup permanently closes volume acquisition
and freezes the complete canonical lease/fence set, including an empty set.

# Responsibilities

The repository locks and matches command identity before creating a run,
validates every state transition, and writes the transition and bounded event
in the same transaction. It captures runtime-Git provenance and loads the
exact release artifacts, parameters, mailbox event, and update context needed
by local materialization. Authorization and actor context are applied at the
database boundary, while storage errors become typed repository errors. This
prevents duplicate NATS deliveries or stale workers from changing another run's
resources or outcome.

The cleanup adapter accepts only the configured worker role and exact provider
scope. It records immutable destruction/absence observations, exposes receipt
lookup for recovery after a crash, and atomically releases the complete set and
completes the run. Public receipt values alone do not prove physical cleanup;
trusted provider integration must establish that observation first. Replaying a
completed receipt never changes a resource's subsequent lease generation.

Historical VM labels without proven host/provider ownership remain unresolved.
Known ownership with a lease on another host fails without releasing resources.
Run-null leases and already-cleaned historical rows receive no invented proof.

The opt-in `create_run_with_vm_plan` requires a configured worker owner scope.
It validates new published pins and the released state requirement, compares
complete inbox inputs on replay, and commits creation, VM plan and events
atomically. The ordinary `create_run` keeps its historical behavior. Fresh
update hooks carry immutable IO prohibition and exact producer admission;
worker planning consumes that evidence once. Closure before consumption allows
cleanup-only planning without current source access, and never opens Start or
acquisition. Neither admission nor a plan proves physical absence.
These additions depend on private ordered 0108 SQL; application enablement and
public migration placement remain pending.

# When

The prospective Legacy placement extension is explicit constructor configuration
via `with_legacy_vm_placement`. It requires LegacyScalar and a checked actual
managed provider scope. Call the separate placement creation port; ordinary
`create_run` refuses this configuration rather than silently making an unplaced
Run. The new private `0108_legacy_vm_placement.sql` loads after the profile,
scalar and lifecycle fragments. It is not a published migration.

Normal birth commits consumed placement atomically; fresh mailbox/update
producers stamp unconsumed eligibility. Worker consumption checks all frozen
pins and scope, and can occur once. Closed-before-consumption cleanup requires
positive never-IO evidence and globally zero lease history; cleanup-only can
never authorize provisioning. Scoped closure retains every fence. Read-only
inventory includes VM-NULL plans, original released tuples, eligible plans and
identifier-only unknown/foreign holds. It does not authorize automatic cleanup
or protect the separate old scalar recovery loop. Core algorithm and application
composition integration remain pending, with no physical or recipe proof.

Construct the repository from the control-plane pool and inject it into
`RunOrchestrator`:

```rust
let repository = PgRunRepository::new(pool);
```

The orchestrator receives committed `Run` values and ordered event evidence;
recovery callers can enumerate non-cleaned runs for reconciliation.

The cleanup adapter and its private 0108 SQL fragment await coordinated migration
publication and runtime integration. Existing orchestration does not yet invoke
these contracts. The ignored `cleanup` integration target currently loads the
coordinated private SQL drafts in a disposable PostgreSQL database; its temporary
loader must become the ordered migrator before publication. It is authored
verification, not evidence of provider destruction or guest unmounting.

The ordinary constructor retains the public107 SQL shape. Explicit
`with_execution_profile` selects fresh producer SQL for the reviewed profile
schema; owned admission also requires the exact VM owner scope and planned
creation path. Replays compare the original profile and never promote old
rows.

The explicit profile lifecycle extension additionally requires the private
`0108_run_profile_lifecycle.sql` fragment after the scalar qualification
fragment. It locks consumer, Run, globally sorted volumes, then leases. Legacy
acquisition commits the immutable scalar tuple first; binding checks that exact
tuple and records the VM identity before provisioning. Canonical binding keeps
the actual planned owner and the complete frozen selection, and projects only
a singleton proven LegacyOrigin into the old scalar tuple.

New IO transitions require open gates, the released pins and exact live binding.
CleaningUp permanently closes acquisition. Scalar terminal transitions require
no globally held Run lease and either the exact released tuple, a positively
stateless bound VM, or a positive pre-IO empty failure. The existing trusted
scalar provider callback must confirm cleanup first; these rows are not a
canonical absence receipt or recipe Retain/Delete proof. Replay never resets a
resource's newer fence or host. Canonical completion still requires its durable
complete-set receipt.

`profiled_recovery_holds` exposes only stable Run IDs and closed reason values.
Explicit Legacy automatic Run recovery is held until physical placement can be
proven; a lease host does not establish VM root ownership, and stateless Runs
have no lease host at all. Owned inventory requires the exact configured scope.
This inventory does not protect the separate old core scalar stale-lease loop;
profile-aware recovery composition is still required before application
activation. No historical VM UUID fallback or owner adoption is authorized.

The new private lifecycle fixtures exercise real PostgreSQL repositories and
the production core with a controlled VM provider. They do not establish
physical VM destruction, guest mount enforcement or application readiness.

Strict Legacy readback additionally validates current open placement before new
IO and exact closed terminal evidence for callback replay. Permanent closure IDs
in inventory are classification only, a checked subset of exact local placements.
Historical/foreign rows never become absence evidence or a new birth plan.

## Explicit qualified Invocation consumption

`with_qualified_invocation_configuration` requires the private117 consumption
schema and positively admitted private115 Invocation producer. The default is
OFF. Explicit owned canonical profile, exact configured provider and immutable
Start payload are required; neither table presence nor a matching tuple adopts
history. The worker records one consumption and system event and processes the
existing Start inbox atomically; the existing Run is returned with `created=false`.

Immutable117 hydration/inspection preserves producer data after Remove or source
withdrawal. Consumption replay and the no-Git catalogue separately recheck the
original invoking actor's current active Execute/SourceUse, exact configuration,
readiness and closure. Start/catalogue require exact queued/version0; progressed
work remains held until qualified lifecycle/full-fence proof exists. Genuine
canonical completed cleanup may replay without IO under its exact117 proof.
The catalogue returns saved parameters and release artifacts, with no Git, mailbox or update context. Lease-held replay is held until
a full frozen-set predicate exists. This boundary enables no named lease,
progress, session, mount or VM operation; global named dispatch remains closed.

## Qualified Invocation lease/progress metadata

The explicit119 configuration requires the unpublished qualified119 schema
after113 through117. Default composition does not enable Invocation leases or
progress. Each operation compares the checked server configuration with the
protected producer and current full binding/grant set; configuration labels
do not prove physical VM or volume ownership. Acquisition freezes the whole
set, including zero, and exact own-fence replay never resets generations.

This slice supports queued admission through Provisioning and immutable held
inventory after execution rights are lost. Starting/Running, sessions, mounts
and actual VM execution remain unsupported. Cleanup cannot clear held fences
without the existing canonical provider and full-set completion receipts.
The global named-dispatch helper stays false.
