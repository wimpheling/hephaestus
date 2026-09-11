# Complete current GCP Cooking CI validation

Owner: unassigned

## Outcome

Finish operational validation of the current private GCP Cooking workflow for
the `hephaestus-508000` project. Preserve the existing Workload Identity
Federation and cleanup controls while proving the diagnostic and full Cooking
paths on the current configuration. This is a follow-up to the scoped MVP-05.1
acceptance and does not reopen or expand that completed task.

## Current evidence

- [x] The protected prebuilt default runner image was built and validated; the
  default repository variable now points to
  `hephaestus-runner-2a7223a74f7b32403ea8f586502b8a3f`.
- [x] The real KVM smoke and private diagnostics path passed in
  [run 34615599394](https://github.com/wimpheling/hephaestus/actions/runs/34615599394),
  including the expected marker, VM absence, authenticated post-delete
  download and credential scan. The retained object and checksum are recorded
  in [`docs/gcp-cooking-ci.md`](../../docs/gcp-cooking-ci.md).
- [x] The diagnostic fixture/quarantine and safe private collection pipeline is
  proven by [run 34593541194](https://github.com/wimpheling/hephaestus/actions/runs/34593541194)
  and the candidate-image diagnostic in
  [run 34609688851](https://github.com/wimpheling/hephaestus/actions/runs/34609688851);
  the runbook records their deliberate fixture, quarantine, cleanup, private
  download and scan evidence. A new focused diagnostic is needed only if the
  corrective change touches collection behavior.
- [ ] A full `gcp-cooking` pass has not yet been accepted. The full attempt in
  [run 34618088312](https://github.com/wimpheling/hephaestus/actions/runs/34618088312)
  failed after 24m39s after all eight diagnostic sources completed and five
  lineage rows were retained. VM cleanup and the private post-delete
  diagnostics download/scan succeeded. The exact failure cause remains under
  investigation; this is useful operational evidence but does not make the
  Cooking run pass.

## Locked controls

- [ ] Keep the existing WIF issuer, repository and workflow restrictions, CI
  service account, private cache/diagnostics buckets, and bucket-scoped access.
- [ ] Keep the current runner image selection and rollback protection under
  repository configuration; do not promote an image from cleanup evidence.
- [ ] Keep disposable VM deletion, absence verification, private diagnostics
  upload/download and credential scanning fail-closed.
- [ ] Do not weaken expected test counts, substitute seeded or application-only
  evidence for the Cooking path, add service-account keys, or claim a dollar
  hard cap from the GCP budget configuration.

## Implementation checklist

- [x] Record the passing default-image and real KVM smoke evidence with links
  and retained artifact/checksum details.
- [ ] Diagnose the exact failure from run 34618088312 using the private
  diagnostics and workflow evidence, preserving the failed status and
  identifying the smallest corrective change.
- [x] Record the accepted diagnostic fixture/quarantine evidence and the
  deliberate failure, startup/runtime evidence, collection and upload, verified
  VM absence, authenticated post-delete download, checksum and credential scan
  from runs 34593541194 and 34609688851.
- [ ] Run one full `gcp-cooking` attempt after diagnosis and record build/update,
  browser, golden, scanner, marker, cleanup and private-artifact evidence.
- [ ] Update the durable runbook and Cooking CI status with the accepted run
  links, exact revisions and any remaining operational limits.
- [ ] Run focused workflow/script checks and `git diff --check`; retain the
  evidence needed before moving this task to `tasks/done/`.

## Non-goals

This task does not change the GCP project, billing or IAM configuration,
redesign the Cooking suite, or make MVP-06 depend on an unreviewed
infrastructure result. If diagnosis shows that a startup fix needs image
compatibility work, a bounded runner-image rebuild may be necessary; promotion
still follows the existing validation and protection rules. Image rebuild
automation remains tracked in
[`automate-gcp-runner-image-rebuilds.md`](../todo/automate-gcp-runner-image-rebuilds.md).

## Completion evidence

Move this task only after a current diagnostic pass and a current full
`gcp-cooking` pass satisfy the workflow's acceptance criteria. Record the exact
GitHub run links, source/image revisions, test result counts, marker and startup
evidence, VM absence timestamps, private object checksum, authenticated
post-delete scan result, and verification commands. A successful cleanup or
private download by itself is not sufficient evidence.
