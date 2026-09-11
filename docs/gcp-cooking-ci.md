# GCP Cooking CI runbook

This runbook is the durable reference for the disposable GCP modes in
[`cooking-e2e.yml`](../.github/workflows/cooking-e2e.yml). The current live
validation is **pending**. Historical smoke or self-hosted Cooking results do
not establish that the current GCP Cooking path is green.

## Configuration

The project is `hephaestus-508000` (project number `84572286146`). Cloud
resources are regional in `europe-west1`; the dispatch selects
`europe-west1-b`, `europe-west1-c`, or `europe-west1-d`, with `b` as the
default. The cloud job has a 50-minute GitHub timeout. The disposable VM
settings are:

| Mode | VM | Storage and lifetime | Identity |
| --- | --- | --- | --- |
| `preflight` | none | reads regional `INSTANCES`; also `N2_CPUS` for a full run | GitHub OIDC CI identity |
| `cache-preflight` | none | verifies cache name, size, MD5 and generation | GitHub OIDC CI identity |
| `diagnostic` | Ubuntu 24.04 `e2-small`, 20 GB `pd-balanced` | no nested KVM; auto-delete disk; 10-minute provider `DELETE` lifetime | runtime service account, `storage-rw` scope |
| `smoke` | Ubuntu 24.04 `n2-standard-8`, 150 GB `pd-balanced` | nested KVM; auto-delete disk; 45-minute provider `DELETE` lifetime | no service account and no scopes |
| `gcp-cooking` | Ubuntu 24.04 `n2-standard-8`, 150 GB `pd-balanced` | nested KVM; auto-delete disk; 45-minute provider `DELETE` lifetime | runtime service account, `storage-rw` scope |
| `image-build` (planned) | disposable SA-less `n2-standard-8` builder | versioned source disk; image capture after stop; explicit cleanup | no service account and no scopes |
| `cooking` | prepared self-hosted `heph-kvm` runner | 30-minute job; local fixture timeout is 1,500 seconds | runner environment, outside GCP |

Quota output proves quota arithmetic, not zonal capacity. The cloud control
script is [`scripts/gcp-kvm-smoke.sh`](../scripts/gcp-kvm-smoke.sh). It labels
each VM with the workflow run, attempt and SHA, checks ownership before
cleanup, and independently describes the exact VM after a delete response.
The provider lifetime is a backstop; cleanup must still run in the workflow
and report verified absence.

The project keeps the existing EUR 10 monthly project-scoped budget with
50%, 80% and 100% actual-spend alert thresholds. Budget notifications are
alerts, not a dollar hard cap and do not stop a running VM. The paid-path
controls are the cache gate, quotas, bounded deadlines, auto-delete disk,
provider `DELETE` lifetime and verified cleanup. See Google's [budgets
documentation](https://cloud.google.com/billing/docs/how-to/budgets).

The full startup budget is 2,100 seconds for bootstrap and Cooking, followed
by a five-minute collection/upload reserve through 2,400 seconds. Diagnostic
startup uses a three-minute trial and an eight-minute collection deadline.
The helper receives the remaining absolute deadline, so bootstrap cannot
reset the Cooking clock. The Cooking command runs as a systemd oneshot with
that remaining deadline as its runtime limit and a bounded stop timeout; this
prevents activation or teardown from extending into the collection reserve.
This deadline behavior remains pending live full-path proof. See
[`scripts/gcp-kvm-startup.sh`](../scripts/gcp-kvm-startup.sh) and
[`scripts/gcp-cooking-run.sh`](../scripts/gcp-cooking-run.sh).

### Planned prebuilt image mode

`image-build` is planned as a manual mode in the existing
[`cooking-e2e.yml`](../.github/workflows/cooking-e2e.yml). Keeping it in this
workflow preserves the current exact WIF provider and `workflow_dispatch`
condition; no WIF change or additional IAM grant is planned. The existing CI
identity already has `roles/compute.instanceAdmin.v1`, which covers the
Compute image and disk operations needed here. A direct SA-less builder also
avoids using the runtime `roles/iam.serviceAccountUser` binding. Google lists
the required image permissions in its [custom image
documentation](https://docs.cloud.google.com/compute/docs/images/create-custom).

Dispatch the planned build from the immutable workflow reference:

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=image-build -f gcp_zone=europe-west1-d
```

The build output will provide an immutable image name of the form
`hephaestus-runner-<first-32-hex-digits-of-manifest-sha>`. The complete
manifest SHA and current recipe, verifier and startup SHA anchors are stored
in the image description; the label carries the shortened name-safe prefix.
That permits an older image to remain usable when its recorded provenance is
compatible with the current recipe, verifier and startup anchors, while the
browser lock and baked browser executable/version are checked again at VM
startup.

After a build is human-reviewed, pass that name to a later mode in the same
workflow. The `runner_image` input is optional for `diagnostic`, `smoke` and
`gcp-cooking`; omit it to use the stock image path. A custom image in
`diagnostic` uses the 150 GB diagnostic disk required by the baked image.

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=diagnostic -f gcp_zone=europe-west1-d \
  -f runner_image=hephaestus-runner-61d34f3bbda2b9cd6c7967249b540595
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=smoke -f gcp_zone=europe-west1-d \
  -f runner_image=hephaestus-runner-<manifest-prefix>
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=gcp-cooking -f gcp_zone=europe-west1-d \
  -f runner_image=hephaestus-runner-<manifest-prefix>
```

The builder will bake the reviewed Rust, libkrun/libkrunfw, passt/AppArmor,
Node and browser dependencies into that versioned image. It will create a
labelled source disk with auto-delete disabled, emit a successful build
marker, stop and delete the builder while keeping the source disk, then create
the image from the detached disk. The provider `DELETE` lifetime is only a
backstop. The image-build path must not place credentials, checkout tokens or
private cache contents in the image. Retain one current image and one rollback
image, and delete older versions after promotion. Custom image storage is
billable; see Google's [disk and image pricing](https://cloud.google.com/compute/disks-image-pricing).

Failed-candidate deletion and recovery by a fresh workflow cleanup step remain
planned acceptance checks until their focused tests pass. No image is
currently claimed as live or approved for `gcp-cooking`.

Build run [34601825193](https://github.com/wimpheling/hephaestus/actions/runs/34601825193)
at commit `15960cdfcfe176e4dc32809b10e7b8eee31b2ada` completed in about
11 minutes. It produced the READY candidate
`hephaestus-runner-61d34f3bbda2b9cd6c7967249b540595`; the full manifest SHA is
`61d34f3bbda2b9cd6c7967249b54059506327db1820aa3a6ae749f500eaf3128`. The
builder, source disk and final cleanup were verified deleted at 13:09:50Z,
13:11:09Z and 13:11:17Z. The local log is
`/tmp/heph-image-build-34601825193.log`. The candidate is not promoted; the
next check is the custom diagnostic dispatch above, while KVM smoke and full
`gcp-cooking` remain pending.

### Keyless identities and bucket access

GitHub authenticates through this workload identity provider:

```text
projects/84572286146/locations/global/workloadIdentityPools/github-actions/providers/github
```

The provider trusts issuer `https://token.actions.githubusercontent.com/` and
uses this claim mapping:

```text
google.subject=assertion.sub, attribute.repository_id=assertion.repository_id, attribute.repository_owner_id=assertion.repository_owner_id, attribute.workflow_ref=assertion.workflow_ref, attribute.ref=assertion.ref, attribute.event_name=assertion.event_name
```

The immutable repository is
`wimpheling/hephaestus` (`repository_id=1312377552`,
`repository_owner_id=471188`). The allowed workflow ref is
`wimpheling/hephaestus/.github/workflows/cooking-e2e.yml@refs/heads/main`;
the exact provider condition from the project bootstrap is:

```text
assertion.repository_id == '1312377552' && assertion.repository_owner_id == '471188' && assertion.ref == 'refs/heads/main' && assertion.workflow_ref == 'wimpheling/hephaestus/.github/workflows/cooking-e2e.yml@refs/heads/main' && (assertion.event_name == 'push' || assertion.event_name == 'workflow_dispatch')
```

The workflow uses `hephaestus-ci@hephaestus-508000.iam.gserviceaccount.com`.
Its reviewed control-plane access includes the project Compute instance
administration role, project `roles/serviceusage.serviceUsageConsumer` for
explicit quota/billing-project requests, and bucket object viewer access.
It has `roles/iam.serviceAccountUser` on the exact runtime service account,
not on the project. No service-account key, GitHub token, or VM credential is
stored in the repository.

The runtime identity is
`hephaestus-cooking-runtime@hephaestus-508000.iam.gserviceaccount.com`.
It has bucket-scoped `roles/storage.objectViewer` on the cache bucket and
bucket-scoped `roles/storage.objectCreator` on the diagnostics bucket. The
runtime VM receives the `storage-rw` Compute access scope because it must read
the private cache and upload one unique diagnostics object. Smoke receives no
identity or scope.

| Bucket | Configuration | Runtime grant | CI grant |
| --- | --- | --- | --- |
| `gs://hephaestus-508000-cooking-cache` | `europe-west1`, STANDARD, uniform bucket-level access, public access prevention enforced, versioning disabled, soft delete `0`, Delete lifecycle at object age 7 days | `storage.objectViewer` | `storage.objectViewer` |
| `gs://hephaestus-508000-cooking-diagnostics` | `europe-west1`, STANDARD, uniform bucket-level access, public access prevention enforced, versioning disabled, soft delete `0`, Delete lifecycle at object age 1 day | `storage.objectCreator` only | `storage.objectViewer` |

The cache object is
`gs://hephaestus-508000-cooking-cache/cooking/heph-gcp-cooking-cache.tar.zst`.
The reviewed immutable archive SHA-256 is
`0ed20efcc1aa019b79405d1eed626b13d4702019e9ceeba2bdde54e45ae29296`.
The diagnostics object key is
`cooking/runs/{github_run_id}/{github_run_attempt}/{github_sha}.tar.gz`.
The runtime uses a unique key and cannot read or delete previous bundles.
Lifecycle deletion is retention control, not an immediate deletion guarantee.

The startup pins Rust `1.88.0`, libkrun `v1.19.0` at commit
`9932c4b59d8f891e60c6aba20d22ebb99ceaa8e2`, libkrunfw `v5.5.0`, and passt at
commit `386b5f5472b89769c025f5d5056348532a823b93`. The Cooking helper pins
the Ubuntu guest image to
`docker.io/library/ubuntu@sha256:52df9b1ee71626e0088f7d400d5c6b5f7bb916f8f0c82b474289a4ece6cf3faf`,
Node `v24.16.0` (SHA-256
`d804845d34eddc21dc1092b519d643ef40b1f58ec5dec5c22b1f4bd8fabde6c9`) and
oras `1.3.3` (SHA-256
`9ce999f8d2de03fc03968b29d743077a58783e545e5eaa53917ca177352d0e59`).
The VM checks out the exact workflow SHA supplied in metadata.

## One-time Cloud Shell setup

The reviewed bootstrap artifacts currently live outside this checkout because
the operator must upload and run them from an already authenticated Cloud
Shell. Their reviewed SHA-256 values are:

```text
49025ada6d6303ccfbc69da414919734c512cae9cc34c6ebcc4af1b51582a8e2  cloud-shell-bootstrap.sh
31cd9d6733e5b59c15b9b23ae754d69875de7c1bb3026e375cb6f10d24cdfb08  cloud-shell-cache-bootstrap.sh
360ef30fb30eb8b871c1064f27eedc7735f93233794e6704bf6f294d2e2808bb  cloud-shell-diagnostics-bootstrap.sh
```

Use the Cloud Shell Upload control to place the three files in the shell home,
verify the hashes, and run plan before apply. The base project bootstrap is
the reproducible source for the project APIs, keyless WIF provider, CI
service account, IAM and existing EUR 10 budget:

```sh
sha256sum cloud-shell-bootstrap.sh cloud-shell-cache-bootstrap.sh cloud-shell-diagnostics-bootstrap.sh
bash cloud-shell-bootstrap.sh plan
bash cloud-shell-bootstrap.sh apply
bash cloud-shell-cache-bootstrap.sh plan
bash cloud-shell-cache-bootstrap.sh apply
bash cloud-shell-diagnostics-bootstrap.sh plan
bash cloud-shell-diagnostics-bootstrap.sh apply
```

The cache script enables `storage.googleapis.com`, verifies the project and
bucket configuration, creates or verifies the runtime identity, applies only
the bucket and exact service-account bindings above, and prints the human
upload destination `gs://hephaestus-508000-cooking-cache/cooking/`. Upload the
selected immutable OCI archive through the Cloud Console, then verify its
SHA-256 before dispatching `cache-preflight`.

The diagnostics script applies the separate one-day bucket and its creator /
viewer bindings. It does not modify the cache bucket, budget, VM policy,
GitHub, or local checkout. Both scripts fail closed on an existing conflicting
bucket policy. `plan` is local output; `apply` requires the human's Cloud
Shell browser login. The diagnostics bucket apply is now human-verified: the
one-day bucket exists with the runtime bucket-scoped object creator grant and
the CI bucket-scoped object viewer grant. The diagnostic evidence path and its
live partial-source proof are verified by runs 34586850977 and 34593541194;
full Cooking live pipeline validation remains pending a successful rerun. Do
not run either script with personal local gcloud credentials.

## Dispatch and acceptance gate

Dispatch from the immutable `main` workflow reference. Use this sequence:

1. Run `preflight` and inspect regional quota output.
2. Run `cache-preflight`; a missing or mismatched object must stop before VM
   creation.
3. Run `diagnostic` to exercise intentional test failure, collection, scan,
   private upload, cleanup, post-delete download and checksum verification.
4. After the diagnostic gate passes, dispatch `gcp-cooking` manually for the
   full build, update, browser and Cooking scenario.

The diagnostic gate is proven by [run 34586850977](https://github.com/wimpheling/hephaestus/actions/runs/34586850977)
at commit `c252f0517c147c86d6560c79c403fe7ce6f6d4a4`: it completed from
09:58:05Z to 10:01:16Z, independently verified VM absence at 10:01:09Z,
downloaded the private object, and passed archive/checksum and credential
scans. The object was
`cooking/runs/34586850977/1/c252f0517c147c86d6560c79c403fe7ce6f6d4a4.tar.gz`
(1,251 bytes, SHA-256
`d71d6008640edc47ae846e764942be28421496d0eb694167ad168c861e342f9e`). The
safe status artifact is
`/tmp/heph-diagnostic-34586850977-artifact2/gcp-diagnostics-status.json`.

The live partial-source proof is [run
34593541194](https://github.com/wimpheling/hephaestus/actions/runs/34593541194)
at commit `ddb0920658417fb2bf6538ed7c066c18d8f742ed`. It completed in 3m11s;
the exact `runtime-log` source was quarantined as
`credential-scan-rejected`, safe evidence was retained, the VM was verified
absent before download, and the authenticated private download and scan
passed. The object was
`cooking/runs/34593541194/1/ddb0920658417fb2bf6538ed7c066c18d8f742ed.tar.gz`
(1,296 bytes, SHA-256
`5b4f25d8e32533f25a5f88217483b3c8ce84649fdacf6f18aa895f0f328717f5`). The
safe status artifact is
`/tmp/heph-diagnostic-34593541194-artifact-2/gcp-diagnostics-status.json`.

The full `gcp-cooking` pipeline remains **pending live validation**. A
successful diagnostic test failure is acceptable only when its diagnostics
result passes. For `gcp-cooking`, the test result must also contain the dedicated
`HEPHAESTUS_GCP_COOKING: PASS` marker. A failed test remains failed even when
diagnostics are collected successfully. Do not weaken expected test counts or
convert a missing marker into success.

The latest full attempt [34588821244](https://github.com/wimpheling/hephaestus/actions/runs/34588821244)
at commit `18fff14` timed out with exit `124`. Its raw serial contained a
fixture credential, so the collector at that historical commit failed closed
for the whole bundle.
VM absence was nevertheless proved at 11:00:45Z/11:00:47Z. The resulting
status manifest is not authoritative for cleanup: a later download failure
overwrote its previously verified cleanup state as `cleanup: unverified`; that
status-writing defect is being fixed. Full runs are re-paused.

The current collector quarantines an unsafe individual source, records its
allowlisted `rejectedSources` classification, and can retain an independently
safe lineage/status partial bundle. The retained files are scanned again before
archive creation; a fixture-bearing source is never redacted into the bundle.
The live partial-source behavior is proven by run 34593541194. The historical
attempt establishes neither a denial cause nor snapshot evidence.

The full acceptance evidence must show the exact checked-out SHA, selected
zone, cache object metadata and checksum, build and installation, update
admission, browser journey and golden assertions, scanner success, private
bundle upload, verified VM absence, authenticated post-delete download,
manifest checksums, and a passing credential scan. No current green claim is
made here until that evidence is available from a live rerun.

## Diagnostics and safe inspection

The producer is [`scripts/collect-cooking-diagnostics.py`](../scripts/collect-cooking-diagnostics.py).
It accepts explicit allowlisted source paths, rejects symlink ancestors,
limits each source to 16 MiB and the combined projected evidence to 128 MiB,
limits lineage to 8 MiB / 20,000 rows, and writes a 0600 archive. Browser
summaries use an allowlisted schema. Request/response bodies, headers, cookies,
storage state, credentials and secret values are excluded from the intended
bundle. The producer passed its focused local payload-projection and schema
gate. A rejected raw source is omitted and represented only by a stable
`rejectedSources` label/reason/status entry; `collectionStatus: partial` makes
the result explicit while the final scanner still gates archive creation. The
diagnostic live bundle gate passed in run 34586850977, and the live
partial-source path passed in run 34593541194. Full GCP Cooking validation
still requires the scanner, upload, post-delete download and checksum
evidence below from a successful full run.

On a failed Cooking unit, the helper uses `systemctl show` with an allowlisted
property set. It does not dump `systemctl status` process trees, because those
trees can expose browser or fixture values from child command arguments. The
source-level replacement is reviewed; no new full-run root cause is claimed
from it.

The workflow's `download-diagnostics` path in
[`scripts/gcp-kvm-smoke.sh`](../scripts/gcp-kvm-smoke.sh) downloads with the
CI identity after VM deletion, bounds the archive, validates every member path
and type, extracts only after validation, checks manifest SHA-256 values, and
runs the credential scanner. Use that path for inspection; do not untar an
unverified object. From an authenticated Cloud Shell checkout, replace the
placeholders and invoke the validated download path. The zone is also used to
verify that the exact VM is absent before copying:

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

The helper performs the authenticated copy, bounded archive and member
validation, safe extraction, manifest checksum verification and credential
scan before reporting a pass. The extracted tree is the inspection surface;
the private archive remains the retained evidence.

The small GitHub artifact is a diagnostics status manifest. It records
provider download, archive format, manifest checksum, scanner, and cleanup
outcomes; the workflow result and serial markers carry the Cooking test
outcome. After the authenticated
post-delete download and credential scan, [`summarize-cooking-diagnostics.py`](../scripts/summarize-cooking-diagnostics.py)
adds a bounded `triage` projection: a correlated `denial` with only the exact
allowed stage/class enums and UUID `run_id`, the latest 50 validated attempt
rows, validated `snapshotStatus`, and source counts (`available`, `missing`,
`availableCount`, `missingCount`, `unavailableCount`, `truncatedCount`). The
projection is capped at 64 KiB and carries no log excerpts or request data.
The private bundle remains the canonical evidence: retain its source manifest
and checksums, and accept it only after the post-delete authenticated download,
archive validation and credential scan pass. The small status artifact is a
safe triage projection, not a replacement for that bundle.

To inspect the safe status artifact, open the completed GitHub run's **Summary**
tab, download `gcp-diagnostics-manifest-{run_id}-{run_attempt}`, and inspect it
with `jq`, for example:

```sh
jq '{cleanup,download,scan,triage}' gcp-diagnostics-status.json
```

Start first-denial correlation with `.triage.denial.run_id`, then compare it
with `.triage.attempts[].attempt_run_id`, attempt state and disposition. For a
downloaded and validated private bundle, the same projection can be generated
locally with `python3 -B scripts/summarize-cooking-diagnostics.py /path/to/cooking-diagnostics`;
the helper rejects an unpassed credential scan
and projects a denial only when its UUID correlates with a retained attempt
run. Refresh the one-day diagnostics bucket and seven-day cache lifecycle
configuration if retention policy changes.

Typed broker denials contain static `denial_stage` and `denial_class` fields,
plus bounded run/slot identifiers. Correlate `run_id` with the lineage
`attempt_run_id`, then use event and attempt IDs to distinguish a denied run
from a retry. The active relay revocation event (update `50`) must settle the
same run with one denied decision and no replacement run. Retryable provider
fault events (updates `45` and `46`) intentionally retain one logical event,
two physical attempts, a failed first run and a successful second run. A
broker denial must not be “fixed” by expecting the provider-fault retry shape.
The current unexplained event `0815d784-5963-4411-8077-e0def0ce6b51` cannot be
mapped to update 50: the retained [GCP run 34573078719](https://github.com/wimpheling/hephaestus/actions/runs/34573078719)
log shows that event starting several runs with repeated `secret_broker`
`denied` records, while the run later failed at
[`scenario.rs:1903`](../examples/cooking/tests/scenario.rs#L1903) on the
duplicate-ingress assertion before the update-50 revocation path at
[`scenario.rs:1482`](../examples/cooking/tests/scenario.rs#L1482) could be
exercised. The local retained log is
`/tmp/gcp-cooking-34573078719-artifact/gcp-kvm-smoke-34573078719-1/gcp-kvm-smoke.log`.
Treat the event's mapping and the earlier failure as unexplained until a new
artifact correlates them.

## Failure triage

Start with the workflow status manifest and the serial output inside the
private bundle. Historical runs may also retain a separate serial artifact.
Check the exact commit SHA and `test-mode`, then classify the first failing
boundary:

- **Preflight/cache:** quota, object name, size, MD5, generation, or CI
  permission. No VM should exist after this failure.
- **Provision/startup:** instance creation, startup `FAIL`, dependency or
  checkout failure. Confirm the ownership labels and cleanup describe result.
- **Cooking:** `gcp-cooking` helper phases, immutable cache checksum, build,
  service, browser, golden or scanner output. Preserve the first failure and
  the test exit status.
- **Diagnostics:** collection, redaction/schema validation, archive/upload,
  post-delete download, manifest checksum, or credential scan. A diagnostics
  pass cannot turn a test failure into a pass.
- **Cleanup:** an error response is inconclusive; independently describe the
  exact labelled VM. Accept cleanup only after verified absence.

Keep the serial log, status manifest and private bundle tied to the same run,
attempt and SHA. Do not broaden IAM, add credentials, reuse a VM name, or
retry a paid run before identifying which boundary failed.

Related implementation and evidence notes are in
[`examples/cooking/CI.md`](../examples/cooking/CI.md). The outside-checkout
Cloud Shell artifacts and their configuration are described in the operator
README at `/home/a/.local/share/hephaestus-gcp/README.md`.
