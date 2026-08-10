# Complete operational acceptance for the forge-owned OCI registry

Owner: unassigned

## Outcome

Before a production Hephaestus forge relies on its Zot-backed OCI registry,
prove the deployed registry's protocol compatibility, edge behavior, failure
recovery, credential safety, and operational limits. This task supplies the
production acceptance evidence that is intentionally outside the blueprint
registry implementation task.

## Dependencies

- The forge-owned OCI registry implementation and its trusted publication path
  are recorded in `tasks/done/forge-owned-oci-registry.md`.
- A non-production, production-shaped environment is available with a migrated
  PostgreSQL database, the forge edge, a registry authority and DNS/TLS
  configuration, isolated Zot storage, and operator credentials.
- The registry contains disposable authenticated test namespaces. Tests must
  never use a customer namespace or destructive garbage collection.

## Locked decisions

| Decision | Required behavior |
| --- | --- |
| Certification target | Run the upstream OCI Distribution conformance suite against the deployed, authenticated forge registry for every OCI capability Hephaestus claims. |
| Test authority | Use a disposable registry authority and test credentials with the same routing, token, TLS, and storage topology as production. Never weaken production authorization merely to satisfy a test client. |
| Failure testing | Inject failures at the edge, callback delivery, reconciliation, storage, and token/key boundaries. Assertions must prove fail-closed execution and recovery without corrupting approved lifecycle state. |
| Recovery | Restore PostgreSQL and Zot storage as a coordinated system, run reconciliation before execution resumes, and retain the evidence of any unavailable or missing approved content. |
| Destructive collection | Keep Zot garbage collection disabled. This task does not authorize enabling it; use `tasks/todo/enable-reviewed-zot-garbage-collection.md` for that decision. |
| Production controls | Quotas and alerts are production-only acceptance. They are designed and proven here, but are not prerequisites for the current blueprint registry task. |

## Non-goals

- Reimplementing OCI Distribution behavior or Zot storage internals.
- Replacing the completed local authenticated-Zot platform-builder smoke with
  a general-purpose CI workflow.
- Enabling destructive Zot garbage collection.
- Treating a local mock, fixture, or structurally rendered Caddy configuration
  as evidence for a deployed edge or storage failure drill.

## Implementation checklist

- [ ] **1. Establish a production-shaped acceptance environment**
  - [ ] Provision an isolated registry authority with DNS, trusted TLS
    certificates, forge edge routing, Zot service isolation, and a migrated
    PostgreSQL control plane.
  - [ ] Configure disposable test namespaces, short-lived trusted-worker and
    operator identities, and no credentials that grant access outside those
    namespaces.
  - [ ] Document the exact Zot artifact digest, configuration revision, edge
    revision, storage backend, registry token verification-key identifier, and
    test environment lifetime.
  - [ ] Add an operator-run acceptance entry point that fails safely when the
    target is not explicitly identified as disposable.
  - [ ] Import the exact digest-pinned Ubuntu platform base through the
    controlled internal import path, record its upstream source digest, and
    prove that this does not enable pull-through or arbitrary upstream pulls.
  - [ ] Produce and retain a reviewed platform-builder release record at the
    production-shaped registry authority, including all approved image/index,
    SBOM, provenance, scan, and optional signature referrer digests plus the
    applied stable catalog references.

- [ ] **2. Certify the claimed OCI Distribution contract**
  - [ ] Select and pin the upstream OCI Distribution conformance suite revision
    compatible with the supported OCI 1.1 capabilities.
  - [ ] Run authenticated manifest and blob pull/push conformance against the
    forge edge and record the complete suite output.
  - [ ] Run index, content-negotiation, digest, and referrers conformance for
    each behavior exposed by the forge registry.
  - [ ] Record unsupported upstream test categories and confirm that the edge
    denies their corresponding Zot surfaces rather than silently exposing them.
  - [ ] Add a repeatable regression command or CI job for the supported
    conformance subset, with secrets supplied only by the runtime boundary.

- [ ] **3. Exercise edge, TLS, and storage outage behavior**
  - [ ] Verify that plaintext registry requests are redirected or rejected as
    specified and that valid TLS requests preserve the required OCI response
    headers, registry authority, and bearer challenge parameters.
  - [ ] Verify that malformed or spoofed `Host` and forwarded headers cannot
    change token audience, repository authority, redirects, or generated
    registry URLs.
  - [ ] Exercise Zot restart during pull, push, and referrer retrieval; verify
    persisted content remains digest-addressable after recovery.
  - [ ] Deliberately make the Zot storage backend unavailable during reads and
    writes; verify publication and execution fail closed, unrelated forge reads
    retain their documented availability, and recovery is observable.
  - [ ] Exercise edge and Zot request/body/time limits with interrupted uploads
    and confirm no leaked partial content becomes approved or executable.

- [ ] **4. Exercise notifications, replay, and reconciliation recovery**
  - [ ] Inject duplicate, reordered, delayed, and missed Zot notifications and
    verify idempotent durable inbox handling without duplicate product events.
  - [ ] Simulate callback endpoint outage and recovery; verify replay or
    reconciliation converges product state without trusting an observation as
    approval.
  - [ ] Submit forged or malformed callback observations and verify they are
    rejected, audited safely, and do not disclose unauthorized digest
    existence.
  - [ ] Create missing approved content, orphaned content, and descriptor
    inconsistency scenarios; verify execution is fail-closed and reconciliation
    produces an operator-safe diagnostic.
  - [ ] Record reconciliation recovery timing, retry bounds, and the final
    lifecycle/event evidence for each scenario.

- [ ] **5. Prove key rotation, compromise response, and token revocation**
  - [ ] Rotate registry token verification keys through the configured overlap
    window; prove valid old tokens behave only for their intended bounded
    period and newly issued tokens use the new key.
  - [ ] Simulate signing-key compromise and execute the documented response:
    stop issuance with the compromised key, publish trusted verification
    material, revoke affected authority, and preserve an audit trail without
    exposing key material.
  - [ ] Revoke a project, repository, workload, and operator authority while
    tokens are active; verify new token requests are denied and existing tokens
    are bounded by their short expiry or explicit revocation mechanism.
  - [ ] Verify cross-project and retired-resource tokens cannot pull, push,
    enumerate, or mount content after the relevant authority transition.
  - [ ] Record key identifiers and event timestamps only; never put tokens,
    private signing material, or customer registry paths in acceptance logs.

- [ ] **6. Prove coordinated backup and restore**
  - [ ] Define and automate the backup order for PostgreSQL control-plane state
    and Zot storage, including configuration and verification-key recovery
    material held by the existing secret boundary.
  - [ ] Restore a representative registry and PostgreSQL backup into an
    isolated environment and verify approved manifests and required referrers
    are available only through their immutable digests.
  - [ ] Run reconciliation before permitting publication, catalog exposure, or
    worker execution after restore.
  - [ ] Test a restore with missing blobs and verify affected execution fails
    closed, diagnostics identify the repair path safely, and unrelated approved
    content remains usable.
  - [ ] Record recovery point, recovery time, reconciliation result, and the
    operator decision required before reopening execution.

- [ ] **7. Configure production-only retention, quotas, and alerts**
  - [ ] Review retention-report roots and storage-growth forecasts before
    choosing namespace, total-storage, concurrent-upload, and upload-duration
    limits.
  - [ ] Configure alerts for storage exhaustion, namespace growth, failed or
    slow uploads, auth failures, notification backlog, reconciliation drift,
    failed pulls, and registry unavailability.
  - [ ] Configure safe dashboards and runbooks that expose aggregate/operator
    diagnostics without tokens, private manifest content, or unauthorized
    namespace discovery.
  - [ ] Exercise each alert with a disposable workload and verify routing,
    deduplication, acknowledgement, and recovery behavior.
  - [ ] Keep quotas and alerts disabled or unconfigured in blueprint/local
    environments unless an explicit environment policy opts in.

- [ ] **8. Verify and record operational acceptance**
  - [ ] Run the repository-required quality gate after any acceptance harness or
    deployment-tooling changes.
  - [ ] Record OCI conformance output, real-client compatibility output, edge
    and storage drill evidence, callback/reconciliation evidence, key-response
    evidence, restore evidence, and alert drill evidence.
  - [ ] Review all evidence for credential, key, and tenant-data redaction.
  - [ ] Link the evidence location and any deliberately deferred destructive-GC
    decision before moving this task to `tasks/done/`.

## Completion evidence

- [ ] Record the disposable environment identity, registry authority, pinned
  Zot artifact, storage backend, and token verification-key identifier.
- [ ] Record successful OCI conformance and supported real-client results.
- [ ] Record successful TLS/header, storage outage, callback/replay,
  reconciliation, key-response, and backup/restore drill results.
- [ ] Record the approved production quota, alert, dashboard, and runbook
  configuration with redacted evidence.
- [ ] Record the required quality-gate output and the linked garbage-collection
  follow-up before moving this task to `tasks/done/`.
