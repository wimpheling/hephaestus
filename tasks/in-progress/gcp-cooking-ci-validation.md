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
- [ ] The latest full attempt,
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
