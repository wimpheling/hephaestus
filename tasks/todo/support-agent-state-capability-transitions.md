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
[`reusable-agent-releases-and-instances.md`](reusable-agent-releases-and-instances.md).
Its requirements, invariants, and implementation plan should be elaborated
when work on it begins.

## Implementation checklist

- [ ] Elaborate the state-capability transition problem and agree on its
  lifecycle and safety requirements before implementation.

## Completion evidence

Populate this section after the task has been elaborated and implemented.
