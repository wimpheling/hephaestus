# Automate content-versioned GCP runner image rebuilds

Owner: unassigned

## Outcome

Automatically prepare a candidate GCP runner image in CI when an image input
changes, validate that candidate on a disposable libkrun smoke VM, and promote
it only after the protected candidate gate passes. A failed or stale candidate
must leave the current pointer unchanged; runs that require changed startup
compatibility may remain blocked until a compatible image is promoted. Runtime
jobs must never fall back silently to stock Ubuntu.

## Locked decisions

| Area | Decision |
| --- | --- |
| Workflow entry | Add a `push`-to-`main` path-filtered image job while retaining the existing `workflow_dispatch` `image-build` escape hatch. Do not build on pull requests. |
| Authentication | Keep the pinned `google-github-actions/auth` action, current Workload Identity Federation provider, and current CI service account. The SA-less builder receives no service account or scopes; runtime test VMs retain only the existing diagnostics upload identity. |
| Version identity | Name and select images by a full content fingerprint derived from all bake inputs. Keep repository SHA as provenance, not as the compatibility key, so a documentation-only commit does not rebuild a paid image. |
| Candidate validation | The default candidate gate is image READY verification, custom-image libkrun smoke validation, private artifact upload before VM deletion, post-deletion download/scanning, and confirmed provider-side VM deletion. Run the diagnostic fixture only when the diagnostic path changes or through an explicit diagnostic dispatch. Full Cooking remains an explicit higher-cost check. |
| Promotion | Automatically promote only after the candidate gate passes, with a protected pause available before pointer mutation and a separate manual rollback operation. Retain one current and one rollback image. |
| Lifetime and retries | Keep the 45-minute provider-enforced DELETE lifetime, bounded workflow timeout, one builder at a time, and no automatic retry of paid failures. A later path-filtered commit or manual dispatch may retry after diagnosis. |
| Failure behavior | An explicitly selected stale, missing, non-READY, or incompatible custom image fails closed before VM creation. Stock Ubuntu is used only when `use_stock_image=true` or when no custom image is selected and the controller emits its internal stock selection. |

## Non-goals

This task does not make image creation part of pull-request CI, publish private
OCI caches or checkout credentials into an image, create a persistent runner,
automatically delete the promoted or rollback image, or broaden runtime IAM.
It does not replace the existing disposable VM cleanup or the separate legacy
self-hosted Cooking workflow.

## Image input and content identity

- [ ] Define one canonical content-input list shared by the detector, builder,
  manifest generator, and startup verifier:
  - [ ] `scripts/gcp-runner-image-provision.sh` and its dependency pins for
    Rust, libkrun, libkrunfw, passt, Node, ORAS, and system packages.
  - [ ] `scripts/gcp-runner-image-bake.sh`,
    `scripts/gcp-runner-image-verify.py`, `scripts/gcp-runner-image-manifest.py`,
    `scripts/gcp-kvm-startup.sh`, and `scripts/gcp-passt-preflight.sh`.
  - [ ] `e2e/playwright/package-lock.json` and the locked Playwright/Chromium
    version and browser asset checksum.
  - [ ] The exact Ubuntu base image identity, rather than only the mutable
    `ubuntu-2404-lts-amd64` family name.
- [ ] Generate a canonical sorted input document and SHA-256 content
  fingerprint from file bytes, pins, lock bytes, and base-image identity.
- [ ] Put the full fingerprint in the image manifest/description and use its
  safe prefix in the image name and label. Preserve the existing manifest
  schema or version it explicitly if fields change.
- [ ] Change existing-image lookup to search compatible READY image content
  fingerprints rather than requiring the current repository SHA, while still
  recording and validating repository SHA as build provenance.
- [ ] Require current recipe, verifier, startup, bake, manifest-generator,
  browser-lock, dependency-pin, and base-image anchors at startup; permit an
  older repository SHA only when every runtime compatibility anchor matches.
- [ ] Add focused tests proving input changes invalidate reuse and unrelated
  source/documentation changes reuse the same content-versioned image.

## Automatic CI orchestration

- [ ] Extend the existing `.github/workflows/cooking-e2e.yml` only, preserving
  its exact WIF workflow reference, so an image job runs only on `main` changes
  matching the canonical image-input list; retain `workflow_dispatch` for
  `image-build`, validation, promotion, and retirement.
- [ ] Keep the exact current WIF provider, pinned action revisions,
  `contents: read` plus `id-token: write`, and the existing workflow branch
  guard. Verify the current CI service account can perform the existing
  instance, disk, and image calls before enabling the automatic trigger; ask
  for a narrowly scoped permission change only if that check proves necessary.
- [ ] Serialize image builds with a dedicated concurrency group and reject a
  second active builder. Collapse or skip queued candidates whose content
  fingerprint is already READY; do not implement an automatic retry loop.
- [ ] Keep the builder SA-less, use the existing state file and ownership
  labels for recovery, and preserve the current separate cleanup step with
  failure propagation.
- [ ] Keep the builder job timeout above the 45-minute provider lifetime by a
  bounded margin. Ensure `always()` cleanup can recover a failed or cancelled
  build without deleting a foreign resource, current image, or rollback image.
- [ ] Emit a candidate name, complete fingerprint, source revision, and safe
  build status as job outputs. Do not emit tokens, metadata credentials,
  private cache paths, or raw serial content.
- [ ] Choose and document the automatic pointer mechanism before implementation:
  use a narrowly scoped protected GitHub variable-update credential, or a
  reviewed repository-controlled pointer/output mechanism that the existing
  workflow can consume. Do not assume `contents: write` or the current runtime
  identity can mutate repository variables.

## Candidate validation and promotion

- [ ] After image creation, verify READY status, ownership labels, image name
  versus manifest fingerprint, all external recipe/startup anchors, required
  paths, and dependency/browser pins.
- [ ] Run the existing custom-image smoke mode and require the real libkrun
  result, actual systemd/journal evidence, and private diagnostics upload
  **before** VM cleanup. Delete the VM and verify its absence, then download
  and scan the private archive. Preserve the smoke exit status through
  collection.
- [ ] Run the existing custom-image diagnostic mode only when image/startup
  diagnostic inputs changed or an explicit diagnostic dispatch requests it;
  require its private collector, scanner, upload-before-delete sequence, and
  safe manifest even when the VM fails before the test body starts.
- [ ] Keep full Cooking behind an explicit dispatch or protected promotion
  environment until its duration and artifact acceptance are proven economical
  for automatic candidates.
- [ ] Add an automatic protected promotion operation that, only after the
  candidate gate passes, records the old `vars.GCP_RUNNER_IMAGE` as rollback,
  updates the current pointer using the selected narrow mechanism, and records
  candidate fingerprint/provenance in the review artifact. Permit an explicit
  manual pause before mutation and keep rollback manual and protected.
- [ ] Require promotion to reject candidates with incomplete diagnostics,
  scanner failures, missing private uploads, failed VM deletion, or any stale
  compatibility anchor.
- [ ] Keep `image-retire` explicit and refuse retirement of current or rollback
  images. Require a successful absence check after deletion and retain enough
  metadata to recover a failed cleanup.

## Security and cost acceptance

- [ ] Prove the baked image contains no checkout credentials, runtime service
  account material, private Cooking cache, browser cookies/storage state,
  fixture secrets, SSH private keys, machine identity, or stale cloud-init
  instance state.
- [ ] Preserve guest-agent/startup functionality and regenerate per-instance
  identity on the next boot.
- [ ] Verify that every automatic path has at most one builder and one
  validation VM at a time, uses provider `DELETE`, keeps disk auto-delete and
  cleanup ownership checks, and cannot turn budget alerts into a claimed hard
  spending cap.
- [ ] Preserve the image-builder source disk's deliberate `auto-delete=no`
  requirement through image capture, then explicitly delete and verify the
  owned disk; keep disposable runtime boot disks on their existing auto-delete
  setting; recover failed cleanup from the persisted state file.
- [ ] Record candidate and cleanup status in safe CI artifacts with one-day
  retention; keep private diagnostics in the existing private bucket policy.

## Tests and documentation

- [ ] Add mocked workflow/build tests for path filtering, content-versioned
  reuse, concurrency/duplicate suppression, candidate failure, cancellation,
  automatic-promotion gating, pointer-update failure, cleanup recovery, and
  protected current/rollback deletion.
- [ ] Add focused startup tests for stale-image fail-closed behavior, manifest
  anchor mismatch, prebuilt runtime usability, smoke timeout/tee failure, and
  diagnostics upload before VM deletion followed by post-deletion download.
- [ ] Run shell syntax checks, focused Python suites, workflow linting, and the
  repository handoff quality gate required by `AGENTS.md`.
- [ ] Update `docs/gcp-cooking-ci.md` and `examples/cooking/CI.md` with the
  automatic trigger, candidate gate, automatic promotion pause, rollback, stale-image,
  cleanup, and cost behavior. Keep the manual image-build and stock-image
  escape hatches documented.
- [ ] Record a successful candidate build and smoke artifacts, plus diagnostic
  artifacts when the diagnostic path was exercised, promotion review, rollback
  pointer, and cleanup/retirement checks before moving this task to
  `tasks/done/`.

## Completion evidence

- [ ] Record the triggering commit and content fingerprint, the candidate image
  name, validation run IDs and private artifact object names, promotion/rollback
  review, and confirmed resource deletion.
- [ ] Record the exact commands and exit statuses for focused tests, workflow
  validation, and `cargo dev quality`.
