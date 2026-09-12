# GCP Cooking CI runbook

This runbook is the durable reference for the disposable GCP modes in
[`cooking-e2e.yml`](../.github/workflows/cooking-e2e.yml). The current live
validation is **accepted for the full-evidence gate** through the no-VM recovery
recorded below. Historical smoke or self-hosted Cooking results do
not establish that the current GCP Cooking path is green.

The optional encrypted `diagnostics-triage` export is published at source
`87399f8` with 9 export tests and 90 diagnostics/collector tests; it
revalidates the fixed private bundle before producing a one-day CMS ciphertext
for a local recipient. See the [encrypted diagnostics export guide](gcp-encrypted-diagnostics.md)
for the local key and inspection procedure.

The first real no-VM encrypted-export proof is [run
34670985068](https://github.com/wimpheling/hephaestus/actions/runs/34670985068)
from source `87399f8d5f7de8e7c107df3cb481344666040556`. It created no VM;
although the selected historical run still had a failed gate acceptance, the
encryption steps succeeded. Local CMS decryption, helper revalidation, and
byte-for-byte comparison passed for the 27,897-byte archive with SHA-256
`1bb476f3d7ef3d5cefcf8176e8671ee6830b574a876c78aa9748e8345b8d3156`. The
full GCP acceptance gate remained open at that time; the next paid trial awaited
the cheap diagnostic. The timestamp correction at `4bede2c` accepts the producer's
single-digit UTC hours; the rejected historical raw inputs remain unavailable.

The latest cheap diagnostic [run
34672856178](https://github.com/wimpheling/hephaestus/actions/runs/34672856178)
from source `4463764819082aa0ee56a960d642498598c4552e` passed with expected
gates `42`/`1`/`42`, gate acceptance passed, and the intended `runtime-log`
quarantine. It retained eight safe sources with no unavailable sources and one
failed lineage source. The VM was absent at `04:26:04.221Z`; authenticated
download began at `04:26:06.641Z`, scanning passed at `04:26:08.918Z`, and
the run completed at `04:26:11Z`. The safe manifest is
`/tmp/heph-gcp-diagnostic-34672856178/gcp-diagnostics-status.json`; the
1,809-byte object has SHA-256
`d98bf506eaac3b5b126dfe88c52bd442f930e9b62f3ea5ad29149328220c553d`.
At that point, full acceptance remained open after the cheap diagnostic.

The latest full GCP trial [run
34673076889](https://github.com/wimpheling/hephaestus/actions/runs/34673076889)
from source `4463764819082aa0ee56a960d642498598c4552e` failed after 26m11s
(04:28:13Z–04:54:24Z). VM absence was verified at approximately
04:54:15Z–04:54:17Z before typed download at `04:54:18.556Z`; the download
and credential scan passed, but triage failed with
`triage-projection-failed`. The safe manifest is
`/tmp/heph-gcp-cooking-34673076889/gcp-diagnostics-status.json`; it has no
gate, lineage, object hash, or object size fields, so no workload or browser
failure is inferred. That workflow result remains red because it predates the summary
projection fix; its incomplete status does not provide a workload or
browser failure classification.

The no-VM recovery [run 34674597133](https://github.com/wimpheling/hephaestus/actions/runs/34674597133)
from source `ab59988` recovered the full trial's workload source SHA
`4463764819082aa0ee56a960d642498598c4552e`. All three gates exited `0`, the
startup supervisor exit was `0`, both browser phases passed with `2` reports
and `0` failures, and collection completed with 10 sources, no rejections or
truncation, snapshot status `ok` and 11 rows. The snapshot includes two deliberate expected faults: a malformed model
response and a relay-response loss; it also includes a sampled
`running`/leased-revocation row captured before settlement. The source
asserts same-run failure with no replacement, and later all gates passed; this
is not a claim about a final database snapshot. The retained failure list is
empty. VM absence was verified before
private download and scan, and local encrypted-export decryption validated the
30,077-byte archive with SHA-256
`d798f71f467c9a45f25f6aec3568c993a5e5bb5bfbfde2bd514b42e233fbe153`. The safe
manifest is `/tmp/heph-gcp-triage-34674597133-new/manifest/gcp-diagnostics-status.json`.
This recovered evidence satisfies full acceptance; no additional paid run is
needed. The original full workflow remains red because it ran before the
summary projection fix; it does not provide a workload or browser failure
classification.

The latest local full run is retained at `/tmp/heph-local-network-full.3FSjc8`
from `HEAD` `87399f8` plus uncommitted shell/network changes. It passed 33
34 golden tests (33 passed, 1 ignored), six PostgreSQL tests, both browser phases with
one report each, a 27-file credential scan covering 1,314,388 bytes, and
cleanup verification and the shell post-check wiring. It did not emit a
failure marker because the run succeeded. This is local evidence only. The
full scripts discovery covered 203 scripts and the focused shell/network
validation covered 29 tests; Bash, Python and diff checks passed. No additional paid run is needed; the image-rebuild plan remains plan-only and
the joint user-plan review still gates MVP-06 work. Focused shell tests verify
failure-marker emission separately.

The most recent full [run 34667868345](https://github.com/wimpheling/hephaestus/actions/runs/34667868345)
from source `874a55824d1316952b1c1ae3288ba00e5a9d4619` failed after 29m26s
(02:30:44Z–03:00:10Z) on the promoted image. Workload exit was `1`; both
browser phases passed, and evidence scanning passed for 26 files and 1,382,903
bytes. The lineage and lineage-status sources were rejected with the generic
`source-validation-rejected` class; collection was partial, but upload,
post-delete download and scan passed. VM absence was verified at 03:00:03.309Z.
The private object was 27,897 bytes with SHA-256
`1bb476f3d7ef3d5cefcf8176e8671ee6830b574a876c78aa9748e8345b8d3156`.
Gate validation passed but acceptance failed. The raw `ENOENT` observation is
not causal proof, and caught-confinement panics are not an application-failure
classification. At that time, paid full retries were paused for no-VM
retained-bundle triage improvement and local lineage diagnosis.

An earlier corrected full attempt is recorded below: [run
34663205477](https://github.com/wimpheling/hephaestus/actions/runs/34663205477)
from source `a453d6dd1823fcf91c6934e9e91185b869ece718` failed on the unchanged
promoted image `hephaestus-runner-f285fc2b8157f8053383fc98bcaec83d` after
26m42s (00:55:21Z–01:22:03Z). Workload exit was `1`; the VM was created at
00:55:59Z and absence was verified at 01:21:52Z. The post-delete object lookup
returned 404, so no archive was available, upload status is unknown, the
credential scan did not run, and no actual gate or browser classification was
retained. The coordinator's failure marker matched the inner runtime failure
before outer startup finished collection/upload; this confirms a race that can
preempt collection, while retained logs do not prove the precise guest-kill
timing. A separate deadline trace found `examples/cooking/run.sh` uses the
provided timeout, with `900` only as fallback; the 1,497.935-second interval
from VM creation to workload exit `1` does not prove a budget timeout. The
coordinator fix is published at `d6ee8c25ac07da98407b4dd46b8282a3363965b7` and
passed 177 focused tests. It makes strict outer `FAIL` records include the
revision validated before mode startup; startup is unchanged, so the promoted
image remains compatible. The later full [run
34665815069](https://github.com/wimpheling/hephaestus/actions/runs/34665815069)
from source `9bc0aada35cabdd73b77e13e54d8e2ce9f130774` confirmed the ordering
fix: the workload passed with exit `0` and the evidence phase passed, then
collection reached a typed fatal operation/collection stage before the strict
outer `FAIL` with the expected revision. Its `snapshot-validation` failure
still prevents full acceptance.
The VM was absent at 02:13:04.247Z, but the private object lookup returned 404,
so no archive or credential scan was available. The latest cheap diagnostic
[run 34665616285](https://github.com/wimpheling/hephaestus/actions/runs/34665616285)
passed in 2m51s with the 177-test baseline: expected gates `42`/`1`/`42` and
quarantine passed; VM absence was verified at 01:45:24.466Z before download
and scan at 01:45:28.838Z for 1,813 bytes, SHA-256
`7d638e7699a6448e8c7d10009b940abea17e4ab224f5883f5f24ef19d3692cb6`. The
collector snapshot fix is published at
`e2b5fb4671dbc472cee594d5ddcf868ad8661c5e` with 180 passing tests, including
real 11-row producer snapshots and status-`ok` stages through collection and
summarization. It quarantines only invalid lineage/status, removes the partial
projection, retains other strict safe sources and emits closed rejection
classes, failing closed if none remain valid. Missing lineage is not full
coverage; the next full run must review lineage specifically. The post-fix cheap
diagnostic above passed. At that time, full retries remained paused for no-VM retained-bundle triage
improvement and local lineage diagnosis. No MVP-06 implementation is part of
this work.
Its predecessor [run 34662878781](https://github.com/wimpheling/hephaestus/actions/runs/34662878781)
from source `5c453ea` failed conclusively with workload exit `127` from the
wrapper before browser execution; the scanner ran and failed because no files
were available. The sidecar was valid, but gate acceptance failed. VM absence was verified at 00:53:09Z, and authenticated
post-delete download/scan passed for 2,321 bytes, SHA-256
`2db3b8f884fd8a59664147e4db1134189771e692103dbd423bd027495d82e6f4`.

An earlier full `gcp-cooking` attempt was [run
34659491446](https://github.com/wimpheling/hephaestus/actions/runs/34659491446)
from source `0c77eef` using the promoted image
`hephaestus-runner-f285fc2b8157f8053383fc98bcaec83d`. It ran from 23:49:34Z
on 2026-09-11 through 00:16:42Z on 2026-09-12 (27m08s), and verified VM
absence at 00:16:36Z. The runtime final phase returned evidence exit `1`, and
collection also returned exit `1`, so no diagnostics object was uploaded.
Post-delete download was absent; there is no checksum, scan or triage result.
The historical logs included `HEPHAESTUS_COOKING_TIMEOUT_SECONDS=1500` while
covering npm, browser, build, update and Cooking. Separate tracing confirms
`examples/cooking/run.sh` uses the provided timeout, with `900` only as
fallback; the approximately 1,538-second interval from VM creation to failure
does not prove a budget timeout or the historical cause.
No valid gate or browser classification was retained. The observed caught
confinement panics are not established as the cause. At that point, paid full
retries were paused while local whole-tree post-processing and source-error
policy work proceeded; the corrected follow-up is listed above.

A preceding full `gcp-cooking` attempt was [run
34658390612](https://github.com/wimpheling/hephaestus/actions/runs/34658390612)
from source `672dbf5` using the promoted image
`hephaestus-runner-f285fc2b8157f8053383fc98bcaec83d`. It ran from 23:31:40Z
to 23:34:30Z (2m50s), reached `gcp-cooking` exit `128` before the Cooking
workload and gate sidecar initialized, and verified VM absence at 23:34:20Z.
Collection/upload and private download completed, but post-delete gate
validation failed because the gate source was missing; triage did not run and
no credential-scan pass is claimed. The object has the fixed run/attempt/SHA
prefix; no-VM retriage later verified the partial bundle below. Runtime
git-ownership initialization and missing-gate retention were later addressed;
this earlier attempt remains failed evidence.

No-VM retriage [run 34658990343](https://github.com/wimpheling/hephaestus/actions/runs/34658990343)
for this attempt verified VM absence and passed private download and scan for
1,497 bytes, SHA-256
`c95b17367b547b4881aa3b7a84d800715aeae855778a53016a039238bd3cba14`. The
partial bundle retained six safe sources and recorded missing gate and scanner
results as expected for the early failure; its fallback browser-summary exit
`128` does not establish a browser execution cause.

A preceding full `gcp-cooking` attempt was [run
34650838816](https://github.com/wimpheling/hephaestus/actions/runs/34650838816)
from source `869dd20`, using the validated default runner image
`hephaestus-runner-2a7223a74f7b32403ea8f586502b8a3f`. It ran from `21:44:46Z`
through `22:12:06Z` (27m20s) and returned aggregate workload exit `1`.
Both browser phases completed with 2/2 reports passing and
`report_state: complete`. Collection completed, and authenticated post-delete
download and scan passed for 29,163 bytes, SHA-256
`a247a151c5cbd4129ebb61d5eef2e5fbc8c8c900472630b5c0b4bf9944bf130f`.
The VM was created at `21:45:20Z` and absence was verified at `22:11:57Z`.
The private object is
`cooking/runs/34650838816/1/869dd209ae9569fb078ccc8a2e3a1bb41d4c0be6.tar.gz`.
The evidence-scan result was unexpectedly unavailable/missing and
`runtimeResults` was empty despite source `869dd20`; the executed script and
capture path remained unresolved for that run. This is failed evidence rather than an
accepted Cooking pass; at that time, full GCP validation remained open. The replacement-image
smoke passed; subsequent new-default full attempts failed as recorded above.
No denial observation is treated as the root cause. No-VM triage
[run 34653789352](https://github.com/wimpheling/hephaestus/actions/runs/34653789352)
preserved the failed-run outcome and verified cleanup, but the historical
failure remains unrecoverable from the safe evidence.

The cheap stock-image diagnostic [run 34656728282](https://github.com/wimpheling/hephaestus/actions/runs/34656728282)
at source `fc2e4a7e56ff3d90b0f23ad124b0a3b430871f62` completed from
23:05:58Z to 23:08:58Z (3m). Its finalized gate sidecar recorded workload
`failed`/42, evidence-scan `failed`/1 with `browser-secret-org`, and browser
validation `failed`/42; the expected diagnostic runtime-log quarantine and
the gate-acceptance policy both passed. The sidecar overall and startup
supervisor exits were both 42. VM absence was verified before the
authenticated post-delete download and scan. The private object is
`cooking/runs/34656728282/1/fc2e4a7e56ff3d90b0f23ad124b0a3b430871f62.tar.gz`
(1,768 bytes, SHA-256
`38781dba3569ace67dc72a91b2e1ff71ad98d22ed4aac01bd7c5e297aa2ac7bd`); the
safe status artifact is
`/tmp/heph-diag-34656728282-1789168184/gcp-diagnostics-status.json`.

The follow-up cheap diagnostic [run 34662650437](https://github.com/wimpheling/hephaestus/actions/runs/34662650437)
used source `5c453ea025c9b1df6670085bd324691c404026e5` and the promoted
`f285fc2b8157f8053383fc98bcaec83d` image. VM absence was verified at
00:47:34.021Z before authenticated private download, scan and triage. The
1,810-byte object has SHA-256
`f340874e7557ef9ba89199ba7cedf5153668ceb5201877abb21f2b868ce338cf`; the
finalized expected diagnostic gates were workload `failed`/42, evidence-scan
`failed`/1 with `browser-secret-org`, and browser validation `failed`/42, with
overall and startup supervisor exits both 42. Expected quarantine and gate
acceptance passed. The corrected full trial is recorded at the top of this
runbook; full validation is still open.

The prefix-marker correction is in source revision `a029192`. The current
root-owned sidecar implementation is published at `0c77eef`; its final local
validation passed 169 focused tests. The cheap stock-image diagnostic below passed because
the new startup provenance anchor requires a replacement image. Replacement
candidate build [run 34657052702](https://github.com/wimpheling/hephaestus/actions/runs/34657052702)
at source `672dbf5fe9d1e1bf2cffc9d828913eedfcb79268` completed successfully
with pinned bake and builder VM/disk cleanup. It produced READY image
`hephaestus-runner-f285fc2b8157f8053383fc98bcaec83d` with manifest
`f285fc2b8157f8053383fc98bcaec83d80ac8a0ce03e3e3c91de6de8b6f9efc`; it was
promoted as the current image. Its real KVM smoke [run 34657895009](https://github.com/wimpheling/hephaestus/actions/runs/34657895009)
at source `672dbf5fe9d1e1bf2cffc9d828913eedfcb79268` passed from 23:23:47Z
to 23:29:16Z (5m29s), with the PASS marker at 23:28:08Z, VM absence verified
at 23:29:06Z, and authenticated post-delete download and scan passing for
1,509 bytes, SHA-256
`f8b4144861980f93bbc77947a3d7ba2ca2b423e44cfbf23333ecd8ae023e1c55`.
The private object used the fixed run/attempt/SHA prefix. The former
`2a7223...` image is now the protected rollback; it requires matching startup
recipe provenance, so its pointer alone does not make it compatible with the
new startup anchor. Subsequent new-default full attempts failed as recorded
above. The existing
`hephaestus-runner-2a7223a74f7b32403ea8f586502b8a3f` remains recorded as the
old build reference.

The structured browser capture pipeline is published at commit `1b49264` and
has 123 focused tests, including a real intentional Playwright failure with a
typed source file, line and column. Raw Playwright JSON stays private on the
VM and is excluded from the diagnostics bundle. The safe `.triage.browser`
projection contains typed counts, `report_state`, `observed_phases`,
`passed_phases` and capped `failure_metadata`; the GCP evidence gate requires
complete passing reports for both the initial and post-operation phases.

No-VM recovery [run 34641960369](https://github.com/wimpheling/hephaestus/actions/runs/34641960369)
recovered the post-operation `spec.ts:97` path. The deterministic
time-of-check/time-of-use correction merged in [PR #18](https://github.com/wimpheling/hephaestus/pull/18)
at `eef193d2ab4e2e63aefd4d827c069e93c8a1ee09`, with all three CI checks green;
the combined local validation then passed from `20:28:24.199615Z` through
`20:33:55.363600Z` (5m31.164s): 33 golden tests passed, 1 was ignored and 0
failed; PostgreSQL 6 passed; both browser phases passed with zero failures and
both `initial` and `post-operation` phases observed and passed. Cleanup and the
runtime/cgroup marker passed, as did the whole-tree credential scan across 32
files including the archive. The collector produced complete schema 1 with six
sources and no rejections, and the summarizer reported no failure or retry.
Private evidence is retained at `/tmp/heph-local-cooking-eef193d.PSyYRA`.
This proves the local path only; at that time, full GCP validation remained open.

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
| `smoke` | Ubuntu 24.04 `n2-standard-8`, 150 GB `pd-balanced` | nested KVM; auto-delete disk; 45-minute provider `DELETE` lifetime | runtime service account, `storage-rw` scope for private diagnostics/cache |
| `gcp-cooking` | Ubuntu 24.04 `n2-standard-8`, 150 GB `pd-balanced` | nested KVM; auto-delete disk; 45-minute provider `DELETE` lifetime | runtime service account, `storage-rw` scope |
| `image-build` | disposable SA-less `n2-standard-8` builder | versioned source disk; image capture after stop; explicit cleanup | no service account and no scopes |
| `cooking` | prepared self-hosted `heph-kvm` runner | 30-minute job; local fixture timeout is 1,500 seconds | runner environment, outside GCP |

Quota output proves quota arithmetic, not zonal capacity. The cloud control
script is [`scripts/gcp-kvm-smoke.sh`](../scripts/gcp-kvm-smoke.sh). It labels
each VM with the workflow run, attempt and SHA, checks ownership before
cleanup, and independently describes the exact VM after a delete response.
The provider lifetime is a backstop; cleanup must still run in the workflow
and report verified absence.

Smoke and `gcp-cooking` use the existing runtime service account only for the
private cache and diagnostics bucket permissions inherited from its bucket
roles. Both paths collect and upload private diagnostics, verify VM deletion,
download and scan the bundle afterward, and retain only the one-day safe
status artifact. The image builder remains SA-less. Diagnostics capture the
actual systemd unit log and bounded journal fields; they do not fabricate
Cooking lineage records.

The project keeps the existing EUR 10 monthly project-scoped budget with
50%, 80% and 100% actual-spend alert thresholds. Budget notifications are
alerts, not a dollar hard cap and do not stop a running VM. The paid-path
controls are the cache gate, quotas, bounded deadlines, auto-delete disk,
provider `DELETE` lifetime and verified cleanup. See Google's [budgets
documentation](https://cloud.google.com/billing/docs/how-to/budgets).

The full startup trial is 2,100 seconds (35 minutes), followed by a
five-minute collection/upload reserve through 2,400 seconds (40 minutes);
the provider `DELETE` lifetime remains 45 minutes. The published baseline at
source revision `8fe2e21` has 173 passing Python tests. The follow-up runtime
budget/error-reporting correction is published in that revision; its actual
GNU-timeout mock executable regression was covered by the added tests. It
derives the Cooking timeout from
the remaining 35-minute trial budget minus a 120-second shutdown/evidence
reserve. If 120 seconds or less remain, no systemd unit starts and the helper
emits a typed timeout exit `124`. The existing test assertions and browser
timeouts are unchanged. Diagnostic startup uses a three-minute trial and an
eight-minute collection deadline. The helper receives the remaining absolute
deadline, so bootstrap cannot reset the Cooking clock. Cleanup uses the
existing `run_with_deadline` helper. This deadline behavior remains pending
live full-path proof. Local retained-tree post-processing passed workload exits
0 and 1 plus scanner-rejection cases; actual systemd-run replay was blocked by
host-service/forge availability, so no host provisioning was performed. Live
budget behavior is not yet validated: the earlier `5c453ea` baseline exposed
an external-timeout invocation regression returning `127`, corrected in the
published `8fe2e21` follow-up. The current image fingerprint is unchanged and
the corrected full trial failed as recorded at the top of this runbook. Paid retries are paused for no-VM retained-bundle triage improvement and local
lineage diagnosis.
See
[`scripts/gcp-kvm-startup.sh`](../scripts/gcp-kvm-startup.sh) and
[`scripts/gcp-cooking-run.sh`](../scripts/gcp-cooking-run.sh).

### Prebuilt image mode

`image-build` is an implemented manual mode in the existing
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
`gcp-cooking`; when it is empty, those modes use the optional repository
variable `vars.GCP_RUNNER_IMAGE`. The `use_stock_image` boolean takes
precedence over both and clears image selection, so it explicitly chooses the
stock image. The default candidate variable now points to the confirmed
`hephaestus-runner-f285fc2b8157f8053383fc98bcaec83d`; the protected rollback is
`hephaestus-runner-2a7223a74f7b32403ea8f586502b8a3f` and requires matching
startup recipe provenance. A custom image in `diagnostic` uses the 150 GB diagnostic disk
required by the baked image.

The startup provenance change required a replacement image build and
validation. Candidate `hephaestus-runner-f285fc2b8157f8053383fc98bcaec83d`
was promoted after its real KVM smoke passed. At that time, full validation
remained open, local lineage diagnosis was still in progress, and the next paid
trial awaited the cheap diagnostic. Use the explicit stock-image diagnostic for cheap validation when
testing startup changes.

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=diagnostic -f gcp_zone=europe-west1-d \
  -f runner_image=hephaestus-runner-f285fc2b8157f8053383fc98bcaec83d
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=smoke -f gcp_zone=europe-west1-d \
  -f runner_image=hephaestus-runner-<manifest-prefix>
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=gcp-cooking -f gcp_zone=europe-west1-d \
  -f runner_image=hephaestus-runner-<manifest-prefix>
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=diagnostic -f gcp_zone=europe-west1-d \
  -f use_stock_image=true
```

Retirement also requires an explicit `runner_image`; it refuses the images
held in `vars.GCP_RUNNER_IMAGE` and `vars.GCP_RUNNER_ROLLBACK_IMAGE`, and then
verifies the selected candidate's absence:

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=image-retire -f runner_image=hephaestus-runner-<manifest-prefix>
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

Build run [34608196400](https://github.com/wimpheling/hephaestus/actions/runs/34608196400)
at source commit `aa38a0211d8d86264c337b88e1f0081252f3b9fa` completed and
verified builder VM and source-disk deletion at 14:20:20Z. It produced the
READY candidate `hephaestus-runner-925f650c449e8679825f522d8cef52de`; the
full manifest SHA is
`925f650c449e8679825f522d8cef52dee0f0b9495c00d21dd19ff337a863ae5b`.

Custom-image diagnostic [run 34609688851](https://github.com/wimpheling/hephaestus/actions/runs/34609688851)
at commit `d458f5a618e27ea7558c45ac7bca31e0e285ae1c` passed from 14:22:03Z to
14:26:07Z (4m04). It passed image/readiness checks before the expected fixture
failure, classified the partial `runtime-log` as credential-scan-rejected,
verified VM absence before the private post-delete download, and passed the
archive scan. The object was
`cooking/runs/34609688851/1/d458f5a618e27ea7558c45ac7bca31e0e285ae1c.tar.gz`
with SHA-256
`ed2c9de4d5317a59eb0e5cc486449ad0be643ed08feb86adb62536801bda3136`.
The candidate's custom diagnostic proof remains valid. A newer image is now
the confirmed default below; no rollback image variable is configured.

Build run [34613716791](https://github.com/wimpheling/hephaestus/actions/runs/34613716791)
at source commit `d08fd2d37bf0cf8207867772cf9bfcbbef11da97` completed with the
builder VM and source disk deleted. It produced the READY image
`hephaestus-runner-2a7223a74f7b32403ea8f586502b8a3f`; its full manifest SHA is
`2a7223a74f7b32403ea8f586502b8a3f6a83dbe521b9882a93bfc87d81e341a0`.
The image was then confirmed in `vars.GCP_RUNNER_IMAGE`; after promotion of
`f285fc2b...`, it is the protected rollback and requires matching startup
recipe provenance. All three failed image candidates are retired. Retirement runs
[34616449063](https://github.com/wimpheling/hephaestus/actions/runs/34616449063),
[34616844428](https://github.com/wimpheling/hephaestus/actions/runs/34616844428)
and [34617225580](https://github.com/wimpheling/hephaestus/actions/runs/34617225580)
verified absence; the latest candidate was deleted at 15:37:53Z and verified
absent at 15:37:54Z. The protected default image is unchanged.

Real KVM smoke and private-artifact proof [run 34615599394](https://github.com/wimpheling/hephaestus/actions/runs/34615599394)
passed from 15:21:05Z to 15:26:29Z (5m24), with the smoke marker at 15:25:20Z.
VM absence was verified at 15:26:18Z, and the authenticated post-delete
download and scan passed. The private object was
`cooking/runs/34615599394/1/d08fd2d37bf0cf8207867772cf9bfcbbef11da97.tar.gz`
with SHA-256
`b77d658e52a1fcb4463aa8881416c8dcc3b5507f64b5457a915d31b676ba3f59`.
This proves the smoke path; one full `gcp-cooking` trial was still pending at
that stage.
The earlier stock-image smoke run 34525055454 (14m57) is retained only as an
observational comparison.

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
diagnostic, smoke and GCP Cooking VMs receive the `storage-rw` Compute access
scope because they read the private cache where needed and upload one unique
diagnostics object. The scope is used only with those bucket-scoped roles.

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
full acceptance is recorded by the no-VM recovery above. Do
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

### Reviewed same-repository PR controller

The performance work adds an operator-approved PR input contract to the
trusted workflow. The workflow must be dispatched from `main`, and its WIF
condition remains pinned to
`cooking-e2e.yml@refs/heads/main`. The current branch's workflow changes are
therefore not exercised by GCP until they have been reviewed and promoted to
`main`; no IAM or WIF change is required.

For a reviewed pull request in `wimpheling/hephaestus`, supply all three PR
fields together. The repository ID is the fixed numeric ID `1312377552`.
The controller calls the GitHub API before creating a VM and requires the PR
to be open, based on that repository, and still pointed at the submitted
40-character lowercase head SHA. A mismatch stops before VM creation.

After the implementation is reviewed on `main`, the operator command is:

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=gcp-cooking -f gcp_zone=europe-west1-d \
  -f runner_image=hephaestus-runner-<validated-manifest-prefix> \
  -f pr_number=<open-pr-number> \
  -f pr_repository_id=1312377552 \
  -f pr_head_sha=<exact-pr-head-sha>
```

PR mode requires the reviewed runner image with browser dependencies already
baked. It fails closed on the stock image because installing Playwright
dependencies as root would execute PR-controlled package code. The
controller labels the VM and diagnostics with the validated PR SHA, run ID
and attempt. Startup fetches only that public source SHA and stages
hash-anchored trusted runtime, gate, scanner, browser-summary and timing
helpers from metadata before invoking the workload.

The PR process and its npm lifecycle setup both run as `forge` in delegated
systemd units with the same sandbox. Its cache is staged and frozen
read-only; controller logs, gate files, raw diagnostics, workflow command
files, `/root`, and the trusted runtime paths are inaccessible. Private
per-run state supplies `HOME` and the npm cache. The baked Rust toolchain and
`/home/forge/.cargo` are the only writable host tool paths exposed to the
workload; `/home/forge/.rustup` and the baked browser directory are read-only.
The root runtime resets its own `PATH` to root-owned system directories before
staging or collecting; the forge cargo path is passed only to workload units.
PR units bind a private per-run runtime directory at `/run/user/10001` so the
host user manager and its sockets are not exposed.
`ProtectProc=invisible`, `ProcSubset=pid`, `ProtectSystem`, `ProtectHome`,
`PrivateTmp`, and owner-based IPv4/IPv6 nftables metadata guards prevent
access to controller credentials and the GCE metadata service. The guard is
installed before PR npm/browser hooks and remains active for passt and the
nested guest. Collection and upload stay root-owned and retain only validated
safe projections. Fork execution remains deferred pending a separate trust
and approval policy.

This is a local implementation foundation, not live PR acceptance evidence.
No paid PR dispatch, cloud baseline, performance claim, image promotion or
IAM change is authorized by this section.

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

An earlier default-image diagnostic [run 34645473842](https://github.com/wimpheling/hephaestus/actions/runs/34645473842)
at source `5a8fa3e85737fbf2ba14171d0461cbd898ddd1a4` completed from
20:40:39Z to 20:43:42Z (3m03). It used the default image
`hephaestus-runner-2a7223a74f7b32403ea8f586502b8a3f`, exited with the expected
fixture code 42, quarantined the intended `runtime-log` credential source,
and passed collection, scan, upload and authenticated post-delete download.
VM absence was verified at 20:43:30.842Z; the post-delete download passed at
20:43:36.365Z. The private object is
`cooking/runs/34645473842/1/5a8fa3e85737fbf2ba14171d0461cbd898ddd1a4.tar.gz`
(1,337 bytes, SHA-256
`ffad953ce37b0b2f5546468332479cbf8fcf621f1dae5bf472f0acb57d00e056`). The
safe status artifact is
`/tmp/heph-diagnostic-34645473842.IGA6IN/gcp-diagnostics-status.json`.

The follow-up cheap diagnostic [run 34650524155](https://github.com/wimpheling/hephaestus/actions/runs/34650524155)
at source `869dd209ae9569fb078ccc8a2e3a1bb41d4c0be6` used the default image
in `europe-west1-d` and completed from 21:40:58Z to 21:43:26Z. It exited with
the expected fixture code 42, quarantined the intended `runtime-log` as
`credential-scan-rejected`, and passed VM cleanup, private upload, authenticated
post-delete download and scan. The VM was deleted at 21:43:26Z. The private
object was 1,339 bytes with SHA-256
`866bfa6c9e1fee02526ef2b69458e69182db6f7e590c3c2cf21189054c213654` under the
fixed key
`cooking/runs/34650524155/1/869dd209ae9569fb078ccc8a2e3a1bb41d4c0be6.tar.gz`.
The new typed scanner path is proven locally; this cloud fixture run has an
expected unavailable/missing `evidenceScan` result. The later full trial at
source `869dd20` failed as recorded above.

An earlier custom-image smoke [run 34610546780](https://github.com/wimpheling/hephaestus/actions/runs/34610546780)
failed after 5m31s at `real-libkrun-smoke`; all prebuilt installers were
skipped, and VM absence was verified. Its final report exposed cold-only
revision variables. Source review found and fixed a deterministic prebuilt
instrumentation defect related to those variables, but the live stderr does
not establish that defect as this run's cause. No rerun had passed at that
stage; the later replacement-image smoke passed as recorded above.

The `gcp-cooking` acceptance requirements remain strict: a successful
diagnostic test failure is acceptable only when its diagnostics result passes,
and a full run must contain the dedicated `HEPHAESTUS_GCP_COOKING: PASS`
marker. A failed test remains failed even when diagnostics are collected
successfully. The recovered full evidence above satisfies these requirements;
the original workflow result remains a historical red result because it
predates the summary projection fix. Do not weaken expected test counts or
convert a missing marker into success.

An earlier full attempt [34588821244](https://github.com/wimpheling/hephaestus/actions/runs/34588821244)
at commit `18fff14` timed out with exit `124`. Its raw serial contained a
fixture credential, so the collector at that historical commit failed closed
for the whole bundle. VM absence was nevertheless proved at
11:00:45Z/11:00:47Z. The resulting status manifest is not authoritative for
cleanup: a later download failure overwrote its previously verified cleanup
state as `cleanup: unverified`; that status-writing defect is retained as
historical evidence.

The published collector snapshot fix quarantines only invalid lineage or
status, removes the invalid partial projection, retains other strict safe
sources and emits closed rejection classes; it fails closed if no valid source
remains. Legitimate producer snapshots, including 11-row snapshots and
status-`ok` stages, survive collection and summarization. Missing lineage does
not constitute full coverage; the next full run must review lineage
specifically. Fatal collector errors emit typed stage/reason metadata without
paths or payloads, while the serial filter retains safe diagnostic lines. The
retained files are scanned again before archive creation; a fixture-bearing
source is never redacted into the bundle. The live partial-source behavior is
proven by run 34593541194. The historical attempt establishes neither a denial
cause nor snapshot evidence.

### Root-owned gate sidecars

For `diagnostic` and `gcp-cooking`, startup and the Cooking helper use the
root-owned sidecars `/var/log/hephaestus/cooking-gate-results.json` and
`/var/log/hephaestus/evidence-scan-status.json`. The gate sidecar records the
checked-out `revision`, the gate-helper `script_sha256`, `test_mode`, the
runtime `overall_exit_code`, the startup-observed `supervisor_exit_code`,
`finalized`, and exactly these gates: `workload`, `evidence-scan` and
`browser-validation`. Each gate has only a closed `state`, `exit_code` and
`reason_class` vocabulary. The runtime aggregate and startup-observed exit
codes may differ, such as when startup reaches its outer timeout; preserve
both values when triaging. Finalization converts unfinished gates to
`state: unknown` with `reason_class: unfinished`.

Startup copies the finalized sidecars into the diagnostics input before the
collector runs. The collector rejects extra fields, raw payloads or secrets,
invalid provenance, and invalid gate transitions. The summarizer exposes the
safe projections as `.triage.gateResults`, `.triage.evidenceScan` and
`.triage.runtimeResults`; legacy or missing gate sidecars are explicitly
`unavailable` rather than inferred from log text. The private bundle remains
canonical and is accepted only after VM absence, authenticated post-delete
download, archive validation and credential scanning. The safe status artifact
and diagnostics object follow the one-day diagnostics retention policy; raw
sidecar contents and reports are never published.

The final local sidecar validation passed 169 focused tests. A gate-sidecar
result cannot turn a failed Cooking workload into a pass. The prefix-marker
correction is in source revision `a029192`; its no-VM historical triage is
recorded above.

The accepted full evidence above shows the exact checked-out SHA, selected
zone, build and installation, update admission, browser journey and golden
assertions, scanner success, private bundle upload, verified VM absence,
authenticated post-delete download, manifest checksums and a passing
credential scan.

The previous full attempt [34630967578](https://github.com/wimpheling/hephaestus/actions/runs/34630967578)
at source `23bb3d4` failed at 18:28:19Z after approximately 26m36s. VM
absence was independently verified at 18:28:07.989Z, and private diagnostics
upload, post-delete download and credential scanning passed. The triage step
failed on a Rust timestamp, so this remains failed evidence rather than a
Cooking pass. Follow-up no-VM triage
[34633918356](https://github.com/wimpheling/hephaestus/actions/runs/34633918356)
at `f278dd6` passed all eight sources. The current failure cause is unknown:
expected denial or caught-confinement markers are observations, not proof of a
bug. Later configuration and browser-capture changes addressed the
`test-output` and `browser-summary` projection gap. PR #17
([track-caller correction](https://github.com/wimpheling/hephaestus/pull/17))
was separate. The later full attempt is recorded at the top of this runbook; at that time,
full GCP validation remained open.

The current code also passed a local full Cooking run using the evidence
collector: the golden suite completed with 33 passed and 1 ignored, six
PostgreSQL tests completed in 1.75s, two browser reports passed with zero
failures, and cleanup, same-stream live scanning, and the whole-tree
`check-browser-evidence` scan all passed. The retained local logs are under
`/tmp/heph-local-cooking-20260911`. This validates the local path only; it did
not reproduce the cloud failure.

No-VM triage [run 34635329822](https://github.com/wimpheling/hephaestus/actions/runs/34635329822)
confirms aggregate workload exit `1`. An older `browser-summary` derived the
same exit value, but that does not establish a browser cause. Main
configuration revision `a107bcc` now uses distinct workload and evidence-scan
markers, records browser-report origin, and strictly projects Rust test
results without false panics; 82 focused tests cover the changes. The four
track-caller attributes are merged in
[PR #17](https://github.com/wimpheling/hephaestus/pull/17), with no claim that
the cloud validation is complete.

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

The published browser capture writes one private Playwright JSON report per
phase, then projects only the typed `.triage.browser` fields before collection.
Missing, malformed, partial or nonpassing phase reports fail evidence
validation without turning an unknown report into a fabricated browser cause.

### Shell failure and network diagnostics

The local shell runners emit one safe `HEPH_GCP_SHELL_FAILURE` record for the
first failure. Its operator-facing fields are `script`, `component`,
`operation`, `reason`, `exit_code` and `line`; the allowlisted values identify
the runner (`cooking-run`, `gateway-libkrun-e2e` or `libkrun-integration`),
the failure stage and its closed reason class. The marker excludes commands,
arguments, paths, payloads and error text. The collector retains the fixed
marker, and the summarizer exposes it as a `shell-failure` technical-context
record with safe `source` and `order` lineage. A successful run does not emit
this failure marker; the focused failure tests exercise that path.

For local topology checks, `scripts/canonical-network-snapshot.py` compares
the interface names and IPv4/IPv6 route topology before and after the test.
It preserves route destinations, gateways, flags, metrics, masks, MTU, window,
IRTT, prefixes and interfaces. It validates but omits only the volatile route
`RefCnt` and `Use` counters, so counter churn does not hide topology changes.
Malformed, oversized or unreadable procfs input fails closed. The full local
shell/network validation covered 203 scripts and 29 focused tests; it passed
without weakening assertions.

The evidence-gate projection fix is published at `869dd20`. Its typed
`.triage.evidenceScan` records the specific rule, file class, path digest and
bounded counts, while `runtimeResults` retains all three runtime outcomes. The
existing runtime log still carries its report; no image or startup change is
part of this fix. Local proof in `/tmp/heph-evidence-gate-proof2.a1z3gT`
retained 9,030,549 bytes of source and the exact 8,388,608-byte tail, including
all three outcomes. It rejected an unsafe `browser-secret-org` scanner case
without retaining the raw fixture or path, represented a missing report as
unavailable, and preserved timeout exit `124`. The fix has 86 focused tests,
including a 79-test independent subset. No-VM historical triage [run
34650213803](https://github.com/wimpheling/hephaestus/actions/runs/34650213803)
completed for failed full run `34645906708`: it verified VM absence, downloaded
and scanned the same 29,306-byte object, and confirmed all eight sources and
both browser phases passed. Its historical bundle had unavailable/missing
`evidenceScan` and empty `runtimeResults`, so it cannot recover the original
failing gate. The cloud failure cause remains unresolved.

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

For a retained historical bundle, use the workflow's `diagnostics-triage`
mode. Supply all three source identity fields. `diagnostics_sha` identifies
the trusted `main` controller workflow run; for a PR workload, also supply
`diagnostics_workload_sha` with the PR head SHA used in the object key. The
job validates the selected per-attempt GitHub Actions API record against the
exact workflow path, `main` branch and controller SHA, then checks the exact
disposable VM name project-wide before reading the fixed private object
prefix. It creates no VM and retains only the one-day safe status artifact.

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=diagnostics-triage -f gcp_zone=europe-west1-d \
  -f diagnostics_run_id=34618088312 -f diagnostics_attempt=1 \
  -f diagnostics_sha=1645642605925fa6835291df4a792d3b1123387a
```

For a PR-backed object, append
`-f diagnostics_workload_sha=<exact-pr-head-sha>`; omit it when the workload
SHA is the same as the controller run SHA.

The first live no-VM validation was [run 34626172381](https://github.com/wimpheling/hephaestus/actions/runs/34626172381),
which completed successfully for that source identity. Its safe status
confirmed VM absence, archive download and scanning, and complete collection;
the retry projection retained the failed first attempt and running second
attempt but had no typed failure or denial, so the underlying retry cause
remains unresolved. Inspect `.triage.retry`, `.triage.attempts` and
`.triage.snapshotStatus`; a terminal `HEPH_COOKING_RETRY` marker is retained
even when the periodic lineage snapshot is stale, with per-ID correlation
flags showing which IDs were present in that snapshot.

PR #15 ([commit 31e1d01](https://github.com/wimpheling/hephaestus/commit/31e1d0178bf05dba72a12f168845074f05761fb3))
adds this terminal retry projection; its repository checks passed in [run
34627468291](https://github.com/wimpheling/hephaestus/actions/runs/34627468291).
A separate local PostgreSQL integration reproduced a `READ COMMITTED`
mailbox-recovery race: the pre-fix path classified a cleaned run as retryable,
while the fixed path kept it leased and the next recovery pass settled it as
delivered/completed. All six PostgreSQL tests and the repository quality gate
passed for that correction, which merged in [PR #16](https://github.com/wimpheling/hephaestus/pull/16)
at commit `1cad9ba56a4d780bc94b2a0f65fab4c646b84075`. Its three CI checks and
final quality gate passed: six integration tests against PostgreSQL 17 and
NATS 2.11, 247 Phoenix tests and 98 UI tests. This local race does not
establish the cause of the unexplained GCP failure.

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
attempt and SHA. The accepted recovery above needs no additional paid run.
Do not broaden IAM, add credentials or reuse a VM name for future trials.

Related implementation and evidence notes are in
[`examples/cooking/CI.md`](../examples/cooking/CI.md). The outside-checkout
Cloud Shell artifacts and their configuration are described in the operator
README at `/home/a/.local/share/hephaestus-gcp/README.md`.
