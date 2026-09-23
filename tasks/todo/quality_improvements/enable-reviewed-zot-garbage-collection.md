# Enable reviewed Zot garbage collection

Owner: unassigned

## Outcome

Hephaestus can reclaim unreachable Zot content only after an operator reviews a
bounded retention plan derived from PostgreSQL roots, proves backup restoration,
and explicitly authorizes one auditable collection run. No timer or registry
configuration may delete content automatically.

## Locked decisions

| Decision | Required behavior |
| --- | --- |
| Default | Destructive Zot garbage collection remains disabled. |
| Authority | PostgreSQL defines protected product roots; Zot defines its content graph. Neither source alone authorizes deletion. |
| Review | A collection plan is immutable, content-addressed, time-bounded, and approved by an operator after a fresh reconciliation. |
| Safety | Missing inventory, unknown schema versions, stale roots, incomplete referrer graphs, failed backups, or registry drift fail closed. |
| Execution | The collector receives no general forge authority and can delete only descriptors named by the approved plan. |
| Audit | Plan creation, approval, execution, skipped objects, failures, and post-run reconciliation are durable operator audit records. |

## Non-goals

This task does not add tag-based lifecycle policy, automatic age-based deletion,
cross-project content sharing, or a general-purpose tenant registry cleanup API.

## Implementation checklist

- [ ] **Prove complete retention inputs**
  - [ ] Extend the provider-neutral inventory adapter to enumerate every Zot
    manifest, index, referrer, and blob reachable or orphaned in the supported
    storage backend without relying only on tags.
  - [ ] Model all approved catalog, build, release, agent,
    repository-builder-revision, active-intent, historical-retention, platform,
    and required-referrer roots represented by the product schema.
  - [ ] Reject inventories that are truncated, stale, inconsistent, or from a
    different registry authority/storage generation.
  - [ ] Add focused tests for shared blobs, untagged manifests, nested indexes,
    referrers, interrupted uploads, retired records, and concurrent publication.

- [ ] **Create an immutable reviewed collection plan**
  - [ ] Derive candidates only from a fresh inventory minus a transactionally
    captured root snapshot and include the complete descriptor/blob closure.
  - [ ] Persist the plan digest, registry generation, root snapshot boundary,
    candidate sizes, reasons, expiry, and operator identity without storing
    credentials or manifest bodies.
  - [ ] Require a second fresh reconciliation immediately before approval and
    invalidate the plan when roots or Zot content changed.
  - [ ] Add dry-run output that reports reclaimed logical/physical bytes and
    namespace impact but has no deletion capability.

- [ ] **Gate and execute one collection run**
  - [ ] Add a narrowly scoped operator approval command with existing
    authorization and audit boundaries; prohibit browser and worker approval.
  - [ ] Invoke Zot's supported garbage-collection mechanism only for the exact
    approved plan and prevent concurrent publication during the destructive
    window.
  - [ ] Make interruption retry-safe, preserve partial-run evidence, and never
    broaden candidates during a retry.
  - [ ] Reconcile every protected root after execution and keep publication and
    execution disabled if any approved content is absent.

- [ ] **Prove recovery and operations**
  - [ ] Restore a coordinated PostgreSQL and Zot-storage backup after a staged
    erroneous deletion and verify every approved digest and referrer before
    resuming execution.
  - [ ] Test storage failure, process restart, plan expiry, signing-key
    rotation, token revocation, orphaned content, missing blobs, and safe
    catalog retirement.
  - [ ] Add capacity and collection metrics, alerts, runbooks, and an emergency
    registry-isolation procedure with no automatic fallback deletion.

- [ ] **Verification**
  - [ ] Run upstream OCI Distribution conformance before and after collection.
  - [ ] Run real Buildah, Skopeo, ORAS, Podman, and worker pull tests against
    retained content.
  - [ ] Run `cargo dev quality`.
  - [ ] Record the reviewed plan digest, backup/restore evidence, reclaimed
    bytes, audit IDs, and post-run reconciliation result.

## Completion evidence

Record command output, immutable plan and artifact digests, restore-test
evidence, and the repository quality result here before moving this task to
`tasks/done/`.
