# Qualified Invocation App PostgreSQL acceptance

This new target calls the public App `PgRunAuthorityManager` and
`CanonicalRunLaunchAuthorizer`. It reuses only helper modules from the genuine
native producer suites. The only copied prerequisite is `bindings::establish`;
its original SQLx case is excluded. The target registers 19 new cases.

The fixture runs real published-source Bootstrap and OwnedBirths through actual
claim/preparation/import/completion ports, then actual 114 activation, 115
admission, configured 117 RunPG consumption and 119 lease/session metadata.
Typed bindings have the actual distinct seeded issuer and current app RLS/grant
checks. Every App call uses the exact current persisted Run, checked config,
worker pool, configured RunPG, and matching canonical LocalVolumeStore with
qualified VolumePG run metadata. Full-set acquire and attach are actual VolumePG
worker operations, including explicit zero selections. App reads/preflights use
the actual canonical Local store. Births use genuine mkfs/fsck/file custody.

No physical VM creation, VM mount/start, guest ACK transport, SQLite execution,
WAL retention, provider deadline, or final supervisor integration is proved here.
The ACK calls exercise the trusted-host App API and persisted session/attachment
binding. Physical backings are checked unchanged. Runtime credentials are
compared inside tests without printing bearer values.

## Loader inputs and order

The existing helper loader requires `HEPHAESTUS_PRIVATE_RECIPE_MIGRATIONS=1`.
Public migrations 1–107 resolve from `/home/a/projects/hephaestus/migrations`.
Its private checkpoint chain loads these files in the listed order:

1. 108: `0108_run_volume_leases.profile.sql`,
   `0108_run_cleanup_fragment.profile.sql`,
   `0108_legacy_scalar_compatibility.sql`, `0108_run_profile_lifecycle.sql`,
   `0108_legacy_vm_placement.sql`.
2. 109: `heph-recipe-0109_external_observation_retirement.sql`.
3. 110: `0110_owned_volume_registration.sql`.
4. 111: `0111_owned_volume_backing.sql`.
5. 112: `0112_owned_partial_retention.atomic.sql`.
6. 113: `0113_deployment_execution_preparation.sql`, then
   `0113_owned_first_ready_lease_exclusion.sql` in that checkpoint checksum.
7. 114: `0114_qualified_instance_activation.corrected.sql`.
8. 115: `0115_qualified_instance_invocation.sql`.
9. 116: `0116_owned_ready_retention.sql`.
10. 117: `0117_qualified_invocation_consumption.nowait.sql`.
11. 119: `0119_qualified_invocation_leases.sql` in the registered checksum.
12. Additive helper SQL (unregistered):
    `0119_qualified_invocation_authority.sql`,
    `0119_qualified_invocation_empty_secret.sql`,
    `0119_qualified_invocation_pre_acquisition.sql`.

All private names resolve under `/var/tmp`. The source handoff includes exact
SHA256s in `private-sql-inputs.json`: 18 inputs belonging to the current private
prerequisite set plus the additive pre-acquisition helper. The loader itself
checks the pinned leases119 SHA384 checksum, exact B1/B2/pre-acquisition SHA256s,
and existing function-body/ACL invariance across the additive pre-acquisition
load. It preserves historical public/private migration checksums.

The target deliberately follows this existing prerequisite chain. It does not
load every current private SQL artifact, including the later queued-cleanup
additions, and supplies no compatibility evidence for those skipped additions.
Validation on the full current28 schema remains separate.

## Acceptance and limits

See the handoff `test-matrix.json` for names and exact exercised criteria.
Run this target with `--ignored --test-threads=1` on native PostgreSQL/Local
prerequisites; an ordinary ignored run is not acceptance evidence. Formatting
is the only author-side check. The validation owner must compile, run all19
cases explicitly, and perform the required strict quality checks.

Time assertions use actual App clock readings and actual held PostgreSQL lease
expiry. They verify SQL microsecond precision before identity hashing, the
one-hour bound, and exact pending replay under a changed configured TTL. Prior
pure controlled timestamp probes are distinct evidence and are not counted in
this target.

Withdrawal cases directly observe genuine qualified RuntimeSessionPG `find`
returning Err for existing pending authority, then observe App denial without
replacement, ACK, metadata mutation or handoff deletion. App may reject at its
earlier live-snapshot check. A unique App `find` failure reached after a valid
snapshot is therefore **unmet**, and fail-closed propagation at that specific
line remains source evidence only. No fake repository or fault hook is used.

Supported exact pending revocation is tested after source withdrawal and retains
held fences and immutable ceilings. Generic Expired/no-session scoped handoff
cleanup is **unimplemented** and remains an explicit integration gap. A single
Run's successful pending revoke does not prove any broader cleanup completion.
