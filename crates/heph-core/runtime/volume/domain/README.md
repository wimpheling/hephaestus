# Purpose

`volume-domain` defines bounded named private-volume release declarations and
exact stable resource bindings. It validates controlled guest paths, capacity
claims, required slots, and consumer attachment modes before provider effects.

# Responsibilities

`GuestMountPath` rejects ambiguous paths and protected guest filesystem trees.
The protected roots are `/proc`, `/sys`, `/dev`, `/run`, `/release`, and
`/workspace`, including descendants. Prefix siblings such as `/release-data`
remain valid. Runtime mounting must also prevent symlink redirection and
conflicts with provider-controlled mounts; lexical validation alone cannot
establish attachment safety.
`VolumeSlotDeclaration` carries a bounded minimum capacity and exact read-only
or read-write mode. Complete-set validation rejects overlapping mounts,
duplicate slots, resource reuse, missing required bindings, unexpected bindings,
and mode mismatches. Optional slots may remain unbound.

Attachment authority permits access to file bytes. It does not enforce SQLite
query or table permissions. The legacy generic `Attach` operation authorizes
administrator binding; the consumer runtime mode is a separate declared ceiling
that providers must enforce. These declarations never create grants.

# When

Use this crate when validating a release's volume slots or resolving immutable
bindings to stable volume IDs. Local provider capacity limits, lease recovery,
detach/fencing, and live authorization remain in their existing contracts and
adapters. The legacy `/var/lib/hephaestus` guest mount is allowed.

`effective_volume_slots` adds the enabled historical `state` slot as required
read-write at that legacy path without mutating authored declarations. Its
one-byte minimum preserves compatibility with releases that declared no
minimum capacity; provider capacity rules still apply. Combined catalogs are
validated and sorted. This view grants no authority and rewrites no records.

Typed mount grant scopes pin an instance, immutable revision, release agent,
slot, resource, exact access mode and frozen runtime contract hash. Grant and
revocation IDs reject nil values. A scope is evidence and grants no authority;
the actor adapter verifies its declaration and live two-sided permissions.
Attachment authority does not establish database query permissions.
