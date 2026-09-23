# Support agent state-capability transitions

Owner: unassigned

## Problem

The reusable-agent POC deliberately rejects release updates that change an
instance between stateless and stateful operation. It also rejects a stateful
release update without an update hook, because Hephaestus cannot currently
prove that the existing volume is compatible with the candidate release.

This leaves several unresolved cases:

- a stateless instance updating to a release that requires persistent state;
- a stateful instance updating to a release that no longer uses its volume;
- a stateful release that is compatible with the existing volume but does not
  need a migration hook;
- a later release wanting to reuse a retained volume after an intervening
  stateless revision;
- rejection, abnormal failure, cleanup, retention, and recovery while a
  capability transition is incomplete.

Guessing in any of these cases could lose state, attach incompatible state, or
run the old or candidate revision against a volume it cannot safely use. Until
the lifecycle is designed, the main task records these updates as visible
invalid candidates and does not start an update guest.

This task follows
[`reusable-agent-releases-and-instances.md`](../../../done/reusable-agent-releases-and-instances.md).
The resource model and migration from the implicit optional instance-state
volume to explicit volume slots and bindings are defined in
[`first-class-sqlite-and-s3-object-primitives.md`](first-class-sqlite-and-s3-object-primitives.md).
That task defines zero-or-more named slots, project-owned resources, and exact
revision bindings. Keep this task focused on safely transitioning an instance
from its active revision's bindings to the candidate revision's bindings,
including detach, retain, restore, and recovery decisions. It does not
reimplement the volume provider or resource authorization model. A new writable
attachment is permitted only after the prior writer is proven detached or
fenced; lease expiry alone is insufficient, and uncertainty must fail closed.

## Decisions to elaborate before implementation

- [ ] Define the transition matrix for stateless-to-stateful,
  stateful-to-stateless, compatible reuse, explicit migration, optional
  unbinding, and later reuse of a retained resource.
- [ ] Define which revision remains active and runnable during each phase, how
  bindings and transition state are recorded atomically, and how retries,
  cancellation, crashes, rollback, and concurrent updates are resolved.
- [ ] Define when a resource is retained, detached, restored, or deleted and
  which actor must authorize each choice. A candidate must not receive a
  writable binding while the old revision may still write.
- [ ] Define which compatibility claims require an update hook or provider
  evidence and how an incompatible or unprovable candidate remains visibly
  rejected without starting a guest.

## Implementation checklist

- [ ] Write the transition state machine and failure/recovery table before
  changing runtime behavior.
- [ ] Implement durable, idempotent transition records tied to exact old and
  candidate revisions and their declared resource bindings; use compare-and-set
  or equivalent protection against concurrent updates.
- [ ] Prove old-writer detach or fencing before granting any candidate
  read-write attachment. Keep the old lease and transition recoverable when
  cleanup is uncertain.
- [ ] Cover the transition matrix with real-PostgreSQL and VM/provider
  integration scenarios, including crash/restart, cancellation, failed
  migration, rollback, retained-volume reuse, and denial when detach/fencing
  cannot be proven.
- [ ] Document the final state machine, operator-visible rejection and recovery
  actions, authorization checks, and retained-resource cleanup policy.

## Completion evidence

Record the transition matrix, state-machine and schema version, exact binding
and authorization fixtures, old-writer detach/fencing evidence, failure and
restart scenarios, retained-resource cleanup evidence, test results, and any
follow-up provider work.
