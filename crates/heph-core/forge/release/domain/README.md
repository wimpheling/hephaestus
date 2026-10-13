# Purpose

`release-domain` models the immutable artifacts and project-owned agent
instances that connect Forge output to runtime execution. A release identifies
an exact agent family, version, artifact set, parameters, runtime policy, and
update hook; an instance selects that release through immutable revisions and
repository attachments.

# Responsibilities

The domain validates release versions, artifact paths, ref selectors,
parameters, and content hashes. `RuntimePolicy::resolve` combines the
platform ceiling with a project's selected policy, while revision and update
types preserve compare-and-swap identity, exact run provenance, trigger rules,
and the state transitions needed for safe rollout and recovery.

`ReleaseAgent.volume_slots` carries authored named-volume declarations.
`effective_volume_slots` provides a validated catalog including the enabled
legacy `state` slot without changing immutable release identity or hashes.
Declarations require exact attachment modes and bounded minimum capacities;
they confer no grants. Named-volume persistence, binding, and runtime mounting
are separate integration work, so this domain addition does not establish
multiple attachments or a usable resource-recipe lifecycle.

The same package also defines UI release metadata, installation identities,
browser handoff lifetimes, and content presentation values. Command keys and
input digests make installation, activation, rollback, disable, and remove
operations replayable without changing the selected release unexpectedly.

# When

Use this crate when importing a published release, creating an instance or
attachment, validating a candidate update, or installing a release's UI. Build
the domain values first; application services use their validated IDs, hashes,
policies, and lifecycle states to perform durable commands.

`InstanceVolumeMode` records the immutable legacy or named attachment profile.
It does not replace revision completeness or the live launch gate.
`InstanceRemovalId` names retained permanent closure admission; the ID alone is
not proof of VM destruction, fenced lease cleanup, or terminal tombstoning.
