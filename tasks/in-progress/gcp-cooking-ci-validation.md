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
  A new startup provenance anchor now requires a replacement image; it has not
  yet been built or promoted. A cheap stock-image diagnostic is dispatching,
  and the old default is stale against that anchor once it is published.
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
- [x] The latest default-image diagnostic [run 34645473842](https://github.com/wimpheling/hephaestus/actions/runs/34645473842)
  at source `5a8fa3e85737fbf2ba14171d0461cbd898ddd1a4` completed from 20:40:39Z
  to 20:43:42Z (3m03) with expected fixture exit 42. It quarantined the
  intended `runtime-log` credential source and passed collection, scan, upload,
  VM-absence verification and authenticated post-delete download. The object
  `cooking/runs/34645473842/1/5a8fa3e85737fbf2ba14171d0461cbd898ddd1a4.tar.gz`
  is 1,337 bytes with SHA-256
  `ffad953ce37b0b2f5546468332479cbf8fcf621f1dae5bf472f0acb57d00e056`; VM
  absence was verified at 20:43:30.842Z and download at 20:43:36.365Z. The
  safe manifest is `/tmp/heph-diagnostic-34645473842.IGA6IN/gcp-diagnostics-status.json`.
- [ ] Latest full `gcp-cooking` attempt [run 34650838816](https://github.com/wimpheling/hephaestus/actions/runs/34650838816)
  from source `869dd20` used the validated default image, ran from 21:44:46Z
  to 22:12:06Z (27m20s), and returned aggregate workload exit `1`. Both browser
  phases passed 2/2 with complete reports. Collection completed, and
  authenticated post-delete download/scan passed for 29,163 bytes with SHA-256
  `a247a151c5cbd4129ebb61d5eef2e5fbc8c8c900472630b5c0b4bf9944bf130f`. The VM
  was created at 21:45:20Z and absence was verified at 22:11:57Z. The object
  is `cooking/runs/34650838816/1/869dd209ae9569fb078ccc8a2e3a1bb41d4c0be6.tar.gz`.
  The evidence-scan result was unexpectedly unavailable/missing and
  `runtimeResults` was empty despite source `869dd20`; the executed script and
  capture path are under investigation. Full GCP validation remains open and
  paid runs are paused until the cheap diagnostic and replacement-image
  validation pass. No denial observation is treated as the root cause. No-VM
  triage [run 34653789352](https://github.com/wimpheling/hephaestus/actions/runs/34653789352)
  preserved the failed outcome and verified cleanup, but the historical
  failure remains unrecoverable from safe evidence.
- [ ] Prior full attempt [run 34645906708](https://github.com/wimpheling/hephaestus/actions/runs/34645906708)
  remains failed evidence; its no-VM historical triage is recorded below.
- [x] The prefix-marker correction is recorded in source revision `a029192`.
  The root-owned sidecars are `/var/log/hephaestus/cooking-gate-results.json`
  and `/var/log/hephaestus/evidence-scan-status.json`; they preserve the
  runtime aggregate and startup-observed exits, exact three-gate state and
  reason metadata, and explicit `unknown`/`unfinished` gates. The published
  implementation `345008827434cc5496ab9736751a9c4725b4f454c` passed 167
  focused tests. Startup copies the sidecars before collection, and the safe
  projection exposes `.triage.gateResults`, `.triage.evidenceScan` and
  `.triage.runtimeResults`; one-day retention and post-delete authenticated
  scan remain acceptance requirements.
- [x] Evidence-gate projection fix `869dd20` adds typed `.triage.evidenceScan`
  rule, file-class, path-digest and bounded-count fields, while `runtimeResults`
  retains all three runtime outcomes. The existing runtime log carries its
  report and no image or startup change is involved. Local proof at
  `/tmp/heph-evidence-gate-proof2.a1z3gT` retained 9,030,549 bytes of source
  and the exact 8,388,608-byte tail, including all outcomes; the unsafe
  `browser-secret-org` scanner case was rejected without retaining its raw
  fixture or path, missing report was unavailable, and timeout exit `124` was
  preserved. The fix has 86 focused tests, including a 79-test independent
  subset. No-VM historical triage [run 34650213803](https://github.com/wimpheling/hephaestus/actions/runs/34650213803)
  completed for failed full run `34645906708`: VM absence, download and scan
  passed for the same 29,306-byte object, with all eight sources and both
  browser phases passing. Its historical bundle had unavailable/missing
  `evidenceScan` and empty `runtimeResults`, so it cannot recover the original
  failing gate. The cloud failure cause remains unresolved.
- [x] Root-owned gate sidecars are implemented at source
  `345008827434cc5496ab9736751a9c4725b4f454c` and locally validated with 167
  focused tests. `cooking-gate-results.json` records the checked-out revision,
  helper SHA-256, mode, runtime aggregate exit, startup-observed exit,
  finalized state and exactly the workload, evidence-scan and
  browser-validation gates; unfinished gates become `unknown`/`unfinished`.
  Startup copies the sidecars before collection, and the safe projection
  exposes `.triage.gateResults`, `.triage.evidenceScan` and
  `.triage.runtimeResults`. Acceptance still preserves a failed workload and
  requires VM absence, authenticated post-delete download, archive validation
  and credential scanning.
- [x] Follow-up cheap diagnostic [run 34650524155](https://github.com/wimpheling/hephaestus/actions/runs/34650524155)
  at source `869dd209ae9569fb078ccc8a2e3a1bb41d4c0be6` used the default image
  in `europe-west1-d` and completed from 21:40:58Z to 21:43:26Z with expected
  fixture exit 42. It quarantined `runtime-log` as
  `credential-scan-rejected`, passed cleanup, upload, authenticated post-delete
  download and scan, and deleted the VM at 21:43:26Z. The fixed object key is
  `cooking/runs/34650524155/1/869dd209ae9569fb078ccc8a2e3a1bb41d4c0be6.tar.gz`
  (1,339 bytes, SHA-256
  `866bfa6c9e1fee02526ef2b69458e69182db6f7e590c3c2cf21189054c213654`). The
  cloud fixture's `evidenceScan` result was expected unavailable/missing; the
  typed scanner path is proven only by local proof. The full trial at source
  `869dd20` remains pending.
- [ ] A full `gcp-cooking` pass has not yet been accepted. The full attempt in
  [run 34618088312](https://github.com/wimpheling/hephaestus/actions/runs/34618088312)
  failed after 24m39s after all eight diagnostic sources completed and five
  lineage rows were retained. VM cleanup and the private post-delete
  diagnostics download/scan succeeded. The exact failure cause remains under
  investigation; this is useful operational evidence but does not make the
  Cooking run pass.
- [ ] The later full attempt in
  [run 34630967578](https://github.com/wimpheling/hephaestus/actions/runs/34630967578)
  at source `23bb3d4` failed at 18:28:19Z after approximately 26m36s. VM
  absence was verified at 18:28:07.989Z, and private diagnostics upload,
  post-delete download and scanning passed. Its triage failed on a Rust
  timestamp; follow-up no-VM triage
  [34633918356](https://github.com/wimpheling/hephaestus/actions/runs/34633918356)
  at `f278dd6` passed all eight sources. The exact cause remains unknown:
  expected denial and caught-confinement markers are observations, not proof
  of a bug. Later configuration and browser-capture changes addressed the
  `test-output` and `browser-summary` projection gap. PR #17
  ([track-caller correction](https://github.com/wimpheling/hephaestus/pull/17))
  was separate. That earlier triage left the failure cause unresolved; full
  GCP validation remains open.
- [ ] A prior full attempt,
  [run 34638077629](https://github.com/wimpheling/hephaestus/actions/runs/34638077629),
  used source `bd777de2bd4100a43c22201219892c9b50f3273a`, the default validated
  runner image, and `europe-west1-d`. It ran from `19:18:02Z` through
  `19:45:31Z` (27m29s) and failed in the post-operation browser phase at
  `crates/hephaestus-app/tests/golden.rs:2039`; the initial browser phase
  passed. The workload and browser results failed separately, while evidence
  collection completed and scanning passed. All eight sources were available;
  there was no typed denial or retry. VM absence was verified at
  `19:45:24.681Z`, and private post-delete download/scan passed at
  `19:45:27.828Z` for 27,796 bytes, SHA-256
  `3027116b89450cc0458cc5258c437188bc6600692d9145842230ec9206bfecb0`.
  The raw Playwright report was not retained locally, so its exact assertion
  remains unknown. Keep full GCP validation open pending current GCP
  diagnostics validation; do not retry the paid path until that validation is
  complete.
- [x] The current code passed a local full Cooking run: 33 golden tests passed
  with 1 ignored, six PostgreSQL tests completed in 1.75s, two browser reports
  passed with zero failures, and cleanup, same-stream live scanning, and the
  whole-tree `check-browser-evidence` scan passed. Logs are under
  `/tmp/heph-local-cooking-20260911`; the cloud failure was not reproduced
  locally.
- [x] No-VM triage [run 34635329822](https://github.com/wimpheling/hephaestus/actions/runs/34635329822)
  confirmed aggregate workload exit `1`. An older `browser-summary` derived
  the same exit value, but that does not establish a browser cause.
- [x] Configuration revision `a107bcc` now has distinct workload and
  evidence-scan markers, browser-report origin, and strict Rust test-result
  projection without false panics; 82 focused tests cover the changes. The
  four track-caller attributes were merged in
  [PR #17](https://github.com/wimpheling/hephaestus/pull/17), with all three CI
  checks and the quality gate passed. Cloud validation remains open.
- [x] The structured browser capture pipeline was published in commit
  `1b49264` with 123 focused tests and a real intentional Playwright failure
  proving typed source location. Raw reports remain VM-private and excluded
  from the bundle; `.triage.browser` retains only typed counts, report state,
  observed/passed phases and capped failure metadata. A successful GCP evidence
  gate now requires complete passing reports for both known browser phases.
- [x] No-VM recovery [run 34641960369](https://github.com/wimpheling/hephaestus/actions/runs/34641960369)
  recovered the post-operation `spec.ts:97` path. The deterministic test
  time-of-check/time-of-use correction merged in [PR #18](https://github.com/wimpheling/hephaestus/pull/18)
  at `eef193d2ab4e2e63aefd4d827c069e93c8a1ee09`, with all three CI checks
  green. No assertion, backend count or timeout was weakened. The combined
  local validation passed from `20:28:24.199615Z` through `20:33:55.363600Z`
  (5m31.164s): 33 golden passed, 1 ignored, 0 failed; PostgreSQL 6 passed;
  both browser phases passed with `initial` and `post-operation` observed and
  passed; cleanup and runtime/cgroup markers passed; the whole-tree
  credential scan covered 32 files including the archive; collector schema 1
  was complete with six sources and no rejections; and the summarizer reported
  no failure or retry. Private evidence is at
  `/tmp/heph-local-cooking-eef193d.PSyYRA`. Full GCP validation remains open.
- [x] No-VM historical triage was validated in [run 34626172381](https://github.com/wimpheling/hephaestus/actions/runs/34626172381)
  for run `34618088312`, attempt `1`, and the exact source SHA. It verified
  project-wide VM absence, downloaded and scanned the private object, and
  retained eight safe sources. The projection preserved the failed first
  retry and running second attempt, but contained no typed failure or denial;
  the cause is therefore irrecoverable from that retained safe bundle.
- [x] PR #15 ([commit 31e1d01](https://github.com/wimpheling/hephaestus/commit/31e1d0178bf05dba72a12f168845074f05761fb3))
  preserves the typed `HEPH_COOKING_RETRY` terminal marker independently of
  a stale periodic snapshot, including lookup status, IDs, states, outcome and
  exit fields. Its repository checks passed in [run 34627468291](https://github.com/wimpheling/hephaestus/actions/runs/34627468291).
- [x] A separate local PostgreSQL integration reproduced the
  `READ COMMITTED` mailbox-recovery race: the pre-fix path classified a run as
  retryable, while the fixed path kept it leased and the next recovery pass
  settled it delivered/completed. All six PostgreSQL tests and the quality gate
  passed; the correction merged in [PR #16](https://github.com/wimpheling/hephaestus/pull/16)
  at commit `1cad9ba56a4d780bc94b2a0f65fab4c646b84075`. Its three CI checks and
  final quality gate passed: six integration tests against PostgreSQL 17 and
  NATS 2.11, 247 Phoenix tests and 98 UI tests. This local race
  is not claimed as the cause of the GCP failure.

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
- [x] Review the retained safe evidence for run 34618088312. It contains no
  typed failure or denial from which the historical exact cause can be
  recovered; any further diagnosis must use new evidence and must preserve the
  failed status.
- [x] Record the accepted diagnostic fixture/quarantine evidence and the
  deliberate failure, startup/runtime evidence, collection and upload, verified
  VM absence, authenticated post-delete download, checksum and credential scan
  from runs 34593541194, 34609688851 and 34645473842.
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
