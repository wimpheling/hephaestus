# Cooking E2E runner

This page is the operator guide for local Cooking E2E and the manual disposable
GCP modes. The durable configuration, identity policy, retained evidence and
full historical record are in the [GCP Cooking CI runbook](../../docs/gcp-cooking-ci.md).
The acceptance checklist and open work are in the [GCP Cooking CI validation
task](../../tasks/in-progress/gcp-cooking-ci-validation.md).

Current `gcp-cooking` live validation remains **open**. The promoted custom
image is `hephaestus-runner-f285fc2b8157f8053383fc98bcaec83d`. The most recent
full [run 34667868345](https://github.com/wimpheling/hephaestus/actions/runs/34667868345)
from source `874a55824d1316952b1c1ae3288ba00e5a9d4619` failed after 29m26s with
workload exit `1`; both browser phases and evidence scanning passed (26 files,
1,382,903 bytes), but lineage and lineage-status were rejected as
`source-validation-rejected`, so gate acceptance failed. Collection was
partial, while upload, post-delete download and scan passed; VM absence was
verified at 03:00:03.309Z. The 27,897-byte object SHA-256 is
`1bb476f3d7ef3d5cefcf8176e8671ee6830b574a876c78aa9748e8345b8d3156`. Raw
`ENOENT` and caught-confinement panics do not establish the application cause.
Paid full retries are paused for no-VM triage improvement and local lineage
diagnosis; no new full run is planned. The earlier corrected full
[run 34663205477](https://github.com/wimpheling/hephaestus/actions/runs/34663205477)
from source `a453d6dd1823fcf91c6934e9e91185b869ece718` failed on that image
after 26m42s (00:55:21Z–01:22:03Z), with workload exit `1`. The VM was created
at 00:55:59Z and absence was verified at 01:21:52Z. The post-delete object
lookup returned 404, so no archive was available, upload status is unknown, the
credential scan did not run, and no actual gate or browser classification was
retained. The coordinator marker matched the inner runtime failure before
outer startup finished collection/upload, confirming a race that can preempt
collection; precise guest-kill timing remains unknown. A separate deadline
trace found `examples/cooking/run.sh` uses the provided timeout, with `900`
only as fallback; the 1,497.935-second VM-creation-to-workload interval does
not prove a budget timeout. The coordinator fix is published at
`d6ee8c25ac07da98407b4dd46b8282a3363965b7` and passed 177 focused tests. It
records the validated revision in strict outer `FAIL` results before mode
startup; startup is unchanged, so the promoted image remains compatible. The
coordinator fix was followed by the passed cheap diagnostic. The later full [run 34665815069](https://github.com/wimpheling/hephaestus/actions/runs/34665815069)
from source `9bc0aada35cabdd73b77e13e54d8e2ce9f130774` confirmed the ordering
fix: the workload and evidence phases passed, then collection reached a typed
fatal operation/collection stage before the strict outer `FAIL` with the
expected revision. Its `snapshot-validation` failure prevented full
acceptance. VM absence was
verified at 02:13:04.247Z; the private object lookup returned 404, so no
archive or credential scan was available. The latest cheap diagnostic [run
34665616285](https://github.com/wimpheling/hephaestus/actions/runs/34665616285)
passed in 2m51s with expected gates `42`/`1`/`42` and quarantine; VM absence
preceded download/scan, which passed for 1,813 bytes (SHA-256
`7d638e7699a6448e8c7d10009b940abea17e4ab224f5883f5f24ef19d3692cb6`). The
collector snapshot fix is published at
`e2b5fb4671dbc472cee594d5ddcf868ad8661c5e` with 180 passing tests, including
real 11-row producer snapshots and status-`ok` stages through collection and
summarization. It quarantines only invalid lineage/status, removes the partial
projection, retains other strict safe sources and fails closed if none remain
valid. Missing lineage is not full coverage; the next full run must review lineage
specifically. The post-fix cheap diagnostic above passed. Full retries remain
paused for no-VM retained-bundle triage improvement and local lineage diagnosis;
no new full run is planned. Its predecessor [run 34662878781](https://github.com/wimpheling/hephaestus/actions/runs/34662878781)
from source `5c453ea` failed with workload exit `127` from the wrapper before
browser execution; the scanner ran and failed because no files were available.
Its sidecar was valid but gate acceptance failed. VM absence was verified at
00:53:09Z, and authenticated post-delete download/scan passed for 2,321 bytes
(SHA-256 `2db3b8f884fd8a59664147e4db1134189771e692103dbd423bd027495d82e6f4`).
That result does not establish a full Cooking pass.

The optional encrypted `diagnostics-triage` export is published at source
`4bede2c` with 184 focused tests. It revalidates the fixed private bundle and
retains only a one-day CMS ciphertext for local inspection; no actual no-VM
encrypted-export proof has been recorded yet. Follow the [encrypted export
guide](../../docs/gcp-encrypted-diagnostics.md) and the canonical runbook for
the current paid-run pause and acceptance status.

The cheap diagnostic and real KVM smoke paths have passed with private
post-delete download and credential scanning. The accepted diagnostic
[run 34662650437](https://github.com/wimpheling/hephaestus/actions/runs/34662650437)
used source `5c453ea025c9b1df6670085bd324691c404026e5` and the promoted image;
its expected fixture gates and `browser-secret-org` quarantine passed. Its
1,810-byte private object has SHA-256
`f340874e7557ef9ba89199ba7cedf5153668ceb5201877abb21f2b868ce338cf`.
The canonical runbook retains the complete diagnostic and full-run history.
Paid full retries remain stopped for no-VM retained-bundle triage improvement
and local lineage diagnosis. No global budget-timeout cause is established.

## Manual GCP modes

Manual cloud dispatches run from the immutable `main` workflow reference. They
use GitHub OIDC and the existing `hephaestus-ci` service account; no key,
GitHub token or VM credential is supplied to the guest. The cloud job is
serialized, has a 50-minute GitHub timeout, and skips the self-hosted `cooking`
job.

Run quota and cache checks before a paid VM:

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=preflight -f gcp_zone=europe-west1-d
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=cache-preflight -f gcp_zone=europe-west1-d
```

`preflight` reads regional `N2_CPUS` and `INSTANCES` quota. Quota arithmetic
does not prove zonal capacity. `cache-preflight` verifies the private,
checksum-pinned cache without creating a VM and prints a bounded lookup error
if the object is missing or unreadable.

The cheap deliberate-failure diagnostic uses an `e2-small` VM with no nested
KVM, cache or Cooking build. The stock image uses a 20 GB auto-deleting disk;
a selected custom image uses the 150 GB disk required by that image:

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=diagnostic -f gcp_zone=europe-west1-d \
  -f runner_image=hephaestus-runner-f285fc2b8157f8053383fc98bcaec83d
```

To force the stock Ubuntu image, set `use_stock_image=true`; it takes
precedence over `runner_image` and the repository default:

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=diagnostic -f gcp_zone=europe-west1-d \
  -f use_stock_image=true
```

The `smoke` mode uses the selected runner image and performs the real nested
KVM smoke plus private diagnostics:

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=smoke -f gcp_zone=europe-west1-d \
  -f runner_image=hephaestus-runner-f285fc2b8157f8053383fc98bcaec83d
```

The full GCP Cooking mode should be dispatched only after preflight, cache
verification and the accepted diagnostic/smoke checks:

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=gcp-cooking -f gcp_zone=europe-west1-d \
  -f runner_image=hephaestus-runner-f285fc2b8157f8053383fc98bcaec83d
```

For a retained bundle, use no-VM triage with the exact source run, attempt and
40-character `main` SHA. This creates no VM:

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=diagnostics-triage -f gcp_zone=europe-west1-d \
  -f diagnostics_run_id=RUN_ID -f diagnostics_attempt=ATTEMPT \
  -f diagnostics_sha=FULL_MAIN_SHA
```

The job validates the GitHub Actions record, verifies the exact disposable VM
is absent project-wide, downloads the fixed private object and retains only a
one-day safe status artifact.

## GCP settings and controls

The project is `hephaestus-508000` (number `84572286146`). The selected region
is `europe-west1`; the workflow permits zones `europe-west1-b`, `-c` and `-d`,
with `-b` as the default. Disposable VMs use these settings:

| Mode | VM and disk | Lifetime | Identity |
| --- | --- | --- | --- |
| `diagnostic` | Ubuntu 24.04 `e2-small`, stock 20 GB or custom 150 GB `pd-balanced` | 10-minute provider `DELETE`; disk auto-deletes | runtime service account, `storage-rw` scope |
| `smoke` | Ubuntu 24.04 `n2-standard-8`, nested KVM, 150 GB `pd-balanced` | 45-minute provider `DELETE`; disk auto-deletes | runtime service account, `storage-rw` scope |
| `gcp-cooking` | Ubuntu 24.04 `n2-standard-8`, nested KVM, 150 GB `pd-balanced` | 45-minute provider `DELETE`; disk auto-deletes | runtime service account, `storage-rw` scope |
| `image-build` | SA-less disposable `n2-standard-8` builder | explicit VM, source-disk and image cleanup | no service account or scopes |

The runtime identity is
`hephaestus-cooking-runtime@hephaestus-508000.iam.gserviceaccount.com`.
It has bucket-scoped access for the private cache and diagnostics paths. The
CI identity has the exact service-account-user binding and bucket viewer access
needed by the workflow. IAM and Workload Identity details are maintained in
the [runbook](../../docs/gcp-cooking-ci.md#keyless-identities-and-bucket-access).

The full path checks this private, immutable cache before creating a VM:

```text
gs://hephaestus-508000-cooking-cache/cooking/heph-gcp-cooking-cache.tar.zst
SHA-256: 0ed20efcc1aa019b79405d1eed626b13d4702019e9ceeba2bdde54e45ae29296
```

A missing or unreadable object, or a checksum mismatch, stops before VM
creation. Diagnostics upload to the private bucket under
`cooking/runs/{run_id}/{attempt}/{sha}.tar.gz`. The safe status artifact is
retained for one day; the cache lifecycle is seven days. The private archive
must be downloaded and scanned only through the validated helper.

Cleanup is independent of the provider lifetime. The workflow deletes the VM,
describes the exact owned resource after a delete response, verifies absence,
then downloads and scans diagnostics. A diagnostics pass does not turn a
failed Cooking workload into a pass. The provider `DELETE` lifetime and disk
auto-delete are backstops, not a dollar spending cap.

For `gcp-cooking`, the helper receives the remaining 35-minute workload budget
minus a 120-second shutdown/evidence reserve. With 120 seconds or less left,
no systemd unit starts and the helper emits typed timeout exit `124`. The
collection deadline remains 40 minutes, the provider `DELETE` lifetime remains
45 minutes, and the deadline is not reset after bootstrap. Existing test
assertions and browser timeouts are unchanged. Live full budget behavior
remains unaccepted until a full run proves this path.

The published runtime correction is `8fe2e21` with 173 passing Python tests.
The earlier `5c453ea` baseline exposed an external GNU-timeout invocation
regression returning `127`; `8fe2e21` corrected that regression.

## Image selection and retirement

`image-build` is implemented in this workflow and keeps the existing exact WIF
provider restriction. Build a versioned candidate with:

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=image-build -f gcp_zone=europe-west1-d
```

The build output supplies an image named
`hephaestus-runner-<first-32-hex-digits-of-manifest-sha>`. The full manifest
SHA and recipe, verifier and startup provenance anchors are in the image
description. Startup also checks the browser lock and baked browser
executable/version. Review and validate a candidate with `diagnostic` and
`smoke` before selecting it for `gcp-cooking`.

The optional `runner_image` input is supported by `diagnostic`, `smoke` and
`gcp-cooking`. An empty input uses `vars.GCP_RUNNER_IMAGE`; `use_stock_image=true`
clears that selection. The current variable is
`hephaestus-runner-f285fc2b8157f8053383fc98bcaec83d`; the protected rollback is
`hephaestus-runner-2a7223a74f7b32403ea8f586502b8a3f` and requires matching
startup provenance.

Retirement requires an explicit candidate and refuses the configured current
and rollback images before verifying the candidate's absence:

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=image-retire -f runner_image=hephaestus-runner-<manifest-prefix>
```

Keep one current image and one rollback image. Custom image storage is
billable; failed candidates and builder disks require verified cleanup.

## Local self-hosted Cooking

The normal `cooking` mode runs on an x86_64 Linux runner labelled `self-hosted`,
`Linux`, `X64`, `heph-kvm`. Configure the repository variable
`HEPHAESTUS_COOKING_RUNNER_ENV` with an absolute path to an operator-maintained
environment file outside the checkout. It supplies the reviewed digest-pinned
Python/Rust guest images and OCI builder inputs; it contains no Telegram
accounts or production credentials.

The runner must satisfy [`preflight.sh`](preflight.sh): non-root execution,
read/write KVM access, rootless Podman, writable cgroup v2 delegation, the
supported libkrun/libkrunfw and passt toolchain, Rust/musl, Node/npm and
Chromium dependencies. Use disk-backed paths for `TMPDIR` and
`HEPHAESTUS_LIBKRUN_TMP_ROOT`; RAM-backed `/tmp` can exhaust memory during OCI
verification. The entry point defaults these paths to `/var/tmp`.

The workflow runs the same `examples/cooking/run.sh` entry point as local
execution and performs real builds, installations, updates and the browser
journey. The browser harness uses the host-side request bridge because
rootless Podman cannot be launched from the VM fixture's mapped user namespace;
each browser phase still runs the actual Phoenix service and Playwright
journey. Same-repository pull requests and `main` pushes use the prepared
runner; fork pull requests do not execute on it.

The self-hosted workflow performs real builds, installations, updates and the
browser journey. It uses a 30-minute job limit and the reviewed local fixture
settings, including `HEPHAESTUS_COOKING_TIMEOUT_SECONDS=1500`. The retained
evidence directory is run-specific under `RUNNER_TEMP`; fixture credentials
cause the evidence check to fail even if matching bytes are redacted from
retained output.

## Safe evidence inspection

Use the runbook's validated `download-diagnostics` path. It verifies VM absence,
archive members, manifest checksums and credentials before extraction. From an
authenticated Cloud Shell checkout:

```sh
gcloud auth list
export GCP_ZONE=europe-west1-b
export GITHUB_RUN_ID=RUN_ID
export GITHUB_RUN_ATTEMPT=ATTEMPT
export GITHUB_SHA=FULL_SHA
export GCP_DIAGNOSTICS_ARCHIVE=/tmp/gcp-diagnostics.tar.gz
export GCP_DIAGNOSTICS_STATUS=/tmp/gcp-diagnostics-status.json
bash scripts/gcp-kvm-smoke.sh download-diagnostics
find /tmp/gcp-diagnostics.tar.gz.extract/cooking-diagnostics -maxdepth 2 -type f -print
jq . /tmp/gcp-diagnostics.tar.gz.extract/cooking-diagnostics/manifest.json
```

Inspect `.triage.gateResults`, `.triage.evidenceScan` and
`.triage.runtimeResults` in the safe manifest. The collector and summarizer
retain bounded typed states, reasons, source counts and validated retry rows;
raw reports, payloads, credentials and unbounded logs are excluded from the
uploaded bundle. Only bounded, scanned sources and safe typed projections are
retained. Both known browser phases must have complete passing reports for full
evidence.

For the complete acceptance checklist, historical run links, failure
classification and retry correlation, use the [canonical runbook](../../docs/gcp-cooking-ci.md)
and [validation task](../../tasks/in-progress/gcp-cooking-ci-validation.md).
