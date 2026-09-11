# Cooking E2E runner

The durable GCP configuration, keyless identities, bucket retention, dispatch
gate and failure triage are in the [GCP Cooking CI runbook](../../docs/gcp-cooking-ci.md).
Current `gcp-cooking` live validation is **pending**; the diagnostic evidence
gate and live partial-source proof have passed, while the results below do not
claim that the full path is green. The latest full run is
[34658390612](https://github.com/wimpheling/hephaestus/actions/runs/34658390612)
from source `672dbf5` using promoted image
`hephaestus-runner-f285fc2b8157f8053383fc98bcaec83d`. It ran from 23:31:40Z
to 23:34:30Z (2m50s), reached `gcp-cooking` exit `128` before workload and
gate sidecar initialization, and verified VM absence at 23:34:20Z. Collection/
upload and private download completed, but post-delete gate validation failed
because the gate source was missing; triage did not run and no credential-scan
pass is claimed. The fixed run/attempt/SHA object prefix exists, but size and
checksum await no-VM retriage. Runtime git-ownership initialization and
missing-gate retention are under investigation; full validation remains open.
A no-VM retriage [run 34658990343](https://github.com/wimpheling/hephaestus/actions/runs/34658990343)
verified VM absence and passed private download and scan for 1,497 bytes,
SHA-256 `c95b17367b547b4881aa3b7a84d800715aeae855778a53016a039238bd3cba14`.
The partial bundle retained six safe sources and recorded missing gate and
scanner results as expected for the early failure; its fallback
browser-summary exit `128` does not establish a browser execution cause.
A preceding full run is
[34650838816](https://github.com/wimpheling/hephaestus/actions/runs/34650838816)
from source `869dd20`, using validated image
`hephaestus-runner-2a7223a74f7b32403ea8f586502b8a3f`. It ran from `21:44:46Z`
to `22:12:06Z` (27m20s) and returned aggregate workload exit `1`. Both browser
phases completed with 2/2 reports passing and `report_state: complete`.
Collection completed, and authenticated post-delete download and scan passed
for 29,163 bytes, SHA-256
`a247a151c5cbd4129ebb61d5eef2e5fbc8c8c900472630b5c0b4bf9944bf130f`. The VM
was created at `21:45:20Z` and absence was verified at `22:11:57Z`. The private
object is
`cooking/runs/34650838816/1/869dd209ae9569fb078ccc8a2e3a1bb41d4c0be6.tar.gz`.
The evidence-scan result was unexpectedly unavailable/missing and
`runtimeResults` was empty despite source `869dd20`; the executed script and
capture path are under investigation. This is failed evidence rather than an
accepted Cooking pass; full GCP validation remains open. The replacement-image
smoke passed; the subsequent new-default full attempt failed early as recorded
above. No
denial observation is treated as the root cause. No-VM triage
[run 34653789352](https://github.com/wimpheling/hephaestus/actions/runs/34653789352)
preserved the failed-run outcome and verified cleanup, but the historical
failure remains unrecoverable from the safe evidence.

The cheap stock-image diagnostic [run 34656728282](https://github.com/wimpheling/hephaestus/actions/runs/34656728282)
at source `fc2e4a7e56ff3d90b0f23ad124b0a3b430871f62` completed from 23:05:58Z
to 23:08:58Z. Its finalized gates recorded workload `failed`/42,
evidence-scan `failed`/1 with `browser-secret-org`, and browser validation
`failed`/42; the expected `runtime-log` quarantine and gate-acceptance policy
passed; both overall and startup supervisor exits were 42. VM absence was
verified before authenticated post-delete download and scan. The private
object is
`cooking/runs/34656728282/1/fc2e4a7e56ff3d90b0f23ad124b0a3b430871f62.tar.gz`
(1,768 bytes, SHA-256
`38781dba3569ace67dc72a91b2e1ff71ad98d22ed4aac01bd7c5e297aa2ac7bd`); the
safe status artifact is
`/tmp/heph-diag-34656728282-1789168184/gcp-diagnostics-status.json`.

The latest cheap diagnostic [run 34659146500](https://github.com/wimpheling/hephaestus/actions/runs/34659146500)
at source `0c77eef0d9c0f17123881c3ad9786333da5f74af` used the promoted default
image and completed from 23:43:58Z to 23:48:02Z (4m04). Its finalized gates
recorded workload `failed`/42, evidence-scan `failed`/1 with the typed
`browser-secret-org` rule, and browser validation `failed`/42; overall and
startup supervisor exits were both 42. Expected runtime-log quarantine and
gate acceptance passed. VM absence was verified at 23:47:55Z before private
post-delete download and scan, which passed for 1,808 bytes, SHA-256
`cb8eeca6e50155821ac9b48f52e8ed3c223324eebc5e1f245711fa7eecb5b470` under the
fixed run/attempt/SHA prefix. The new-default full trial remains pending.

The prefix-marker correction is in source revision `a029192`. The root-owned
sidecar implementation is published at `0c77eef`; its final local validation
passed 169 focused tests. The cheap stock-image diagnostic below passed because
the new startup provenance anchor requires a replacement image. Replacement
candidate build [run 34657052702](https://github.com/wimpheling/hephaestus/actions/runs/34657052702)
at source `672dbf5fe9d1e1bf2cffc9d828913eedfcb79268` completed successfully
with pinned bake and builder VM/disk cleanup. It produced READY image
`hephaestus-runner-f285fc2b8157f8053383fc98bcaec83d`; it was promoted as the
current image. Its real KVM smoke [run 34657895009](https://github.com/wimpheling/hephaestus/actions/runs/34657895009)
at source `672dbf5fe9d1e1bf2cffc9d828913eedfcb79268` passed from 23:23:47Z to
23:29:16Z (5m29s), with VM absence verified at 23:29:06Z and authenticated
post-delete download/scan passing for 1,509 bytes, SHA-256
`f8b4144861980f93bbc77947a3d7ba2ca2b423e44cfbf23333ecd8ae023e1c55`. The
former `2a7223...` image is protected rollback and requires matching startup
recipe provenance. The new-default full attempt failed early as recorded
above.

The structured browser capture pipeline is published at commit `1b49264` and
has 123 focused tests, including a real intentional Playwright failure with a
typed source file, line and column. Raw Playwright JSON remains VM-private and
is excluded from the diagnostics bundle. The safe `.triage.browser` projection
contains typed counts, `report_state`, `observed_phases`, `passed_phases` and
capped `failure_metadata`; the GCP evidence gate requires both known browser
phases to pass. Full live validation remains pending.

The evidence-gate projection fix is published at `869dd20`. Its typed
`.triage.evidenceScan` records the specific rule, file class, path digest and
bounded counts, while `runtimeResults` retains all three runtime outcomes. The
existing runtime log still carries its report; no image or startup change is
part of this fix. Local proof in `/tmp/heph-evidence-gate-proof2.a1z3gT`
retained 9,030,549 bytes of source and the exact 8,388,608-byte tail, including
all three outcomes. It rejected an unsafe `browser-secret-org` scanner case
without retaining the raw fixture or path, represented a missing report as
unavailable, and preserved timeout exit `124`. The fix has 86 focused tests,
including a 79-test independent subset. No-VM historical triage for failed full
run `34645906708` completed in [run 34650213803](https://github.com/wimpheling/hephaestus/actions/runs/34650213803): VM absence,
download and scan passed for the same 29,306-byte object, with all eight
sources and both browser phases passing. The historical bundle had
unavailable/missing `evidenceScan` and empty `runtimeResults`, so it cannot
recover the original failing gate. The cloud failure cause remains unresolved.
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
This proves the local path only; full live GCP validation remains pending.

The [Cooking E2E workflow](../../.github/workflows/cooking-e2e.yml) runs the
same `examples/cooking/run.sh` entry point as local execution. It requires an
x86_64 Linux runner labelled `self-hosted`, `Linux`, `X64`, and `heph-kvm`.
The workflow requires real builds and installations, updates, and the cooking
browser journey. Application-only and seeded-release diagnostics do not meet
its acceptance criteria.
The browser harness starts through a host-side request bridge because rootless
Podman cannot be launched from the VM fixture's mapped user namespace. The CI
job first smoke-tests that bridge; each browser phase still runs the actual
Phoenix service and Playwright journey against the cooking daemon.
Automatic pull-request runs are limited to branches in this repository; fork
code does not execute on the prepared self-hosted machine.

The workflow is exercised from a same-repository pull request. The authoritative
current result is the [PR #4 checks page](https://github.com/wimpheling/hephaestus/pull/4/checks);
PR #4 is merged and its checks are green. A reproducible setup uses a fresh
checkout, the checksum-verified official GitHub Actions Linux x64 runner
v2.337.0, an ephemeral `heph-kvm` registration, and the repository variable
below. Runner IDs, online state and temporary
registration directories are operational details, not lasting prerequisites.

The fresh-checkout local run at commit `4f86a30` passed 33 golden tests with no
failures and one ignored test, plus all six PostgreSQL tests and the retained
117-table secret scans. Cooking CI run [34137748510](https://github.com/wimpheling/hephaestus/actions/runs/34137748510)
passed 33 golden tests, six PostgreSQL tests, the 17-file credential scan,
ZIP/archive scan and artifact gate. Generic CI run [34137748509](https://github.com/wimpheling/hephaestus/actions/runs/34137748509)
passed all three jobs. The CI-shaped browser run passed all 12 tests and its
60-file archive scan.

The final quality evidence in `/tmp/heph-mvp05-quality-final-07.log`
(`quality07`) passed Rust 615 tests, Phoenix 247 tests, UI 98 tests and
documentation for 90 files, with the actual PostgreSQL/NATS services. The
supplementary authorization, update-admission and Phoenix precommit checks
also passed.

The observed KVM result is the successful PR run above. No post-merge `main`
KVM job is claimed from PR checks alone.

## Disposable GCE KVM smoke

The same workflow has a manual `cloud_mode` dispatch on `ubuntu-latest`. The
`preflight` mode authenticates with the reviewed GitHub OIDC provider and reads
the `europe-west1` `N2_CPUS` and `INSTANCES` quotas. It reports available quota
but cannot prove zonal capacity. The `smoke` mode creates one Ubuntu 24.04
`n2-standard-8` VM in `europe-west1-b` with nested virtualization, a 150 GB
balanced persistent boot disk, an ephemeral external address, the existing
runtime service account with `storage-rw` scope, and a provider-enforced
45-minute `DELETE` lifetime. That identity is used only for the private cache
and diagnostics bucket permissions inherited from its bucket roles. The disk
is configured for automatic deletion. The VM receives only the exact workflow
commit SHA and the checked-in startup script through metadata; no credentials
or GitHub runner registration token is passed to it.

Smoke always collects and uploads private diagnostics, verifies VM absence,
then performs the authenticated post-delete download and credential scan. The
safe status artifact is retained for one day. It records the actual systemd
unit log and bounded journal fields; it does not fabricate Cooking lineage
records. The image builder remains SA-less.

For a retained historical bundle, the same workflow offers a no-VM
`diagnostics-triage` mode. Pass the exact source run, attempt and `main` SHA:

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=diagnostics-triage -f gcp_zone=europe-west1-d \
  -f diagnostics_run_id=34618088312 -f diagnostics_attempt=1 \
  -f diagnostics_sha=1645642605925fa6835291df4a792d3b1123387a
```

The job validates the per-attempt GitHub API record, verifies the exact
disposable VM name is absent project-wide, then reads the fixed private object
and retains only the one-day safe status artifact. Validation run
[34626172381](https://github.com/wimpheling/hephaestus/actions/runs/34626172381)
passed collection and scanning; it showed the failed first retry and running
second attempt, with no safe typed failure explaining the cause.

PR #15 ([commit 31e1d01](https://github.com/wimpheling/hephaestus/commit/31e1d0178bf05dba72a12f168845074f05761fb3))
adds the typed terminal retry marker used by this projection; its checks
passed in [run 34627468291](https://github.com/wimpheling/hephaestus/actions/runs/34627468291).
A separate local PostgreSQL integration reproduced a `READ COMMITTED`
mailbox-recovery race and validated the correction from retryable to leased,
then delivered/completed on the next pass. All six PostgreSQL tests and the
quality gate passed; the correction merged in [PR #16](https://github.com/wimpheling/hephaestus/pull/16)
at commit `1cad9ba56a4d780bc94b2a0f65fab4c646b84075`, with all three CI checks
green and final quality passing: six integration tests against PostgreSQL 17
and NATS 2.11, 247 Phoenix tests and 98 UI tests. This does not identify the
GCP failure cause.

Ubuntu Noble's packaged `passt` predates the DHCP broadcast behavior required
by libkrun's minimal DHCP client. Before AppArmor setup and passt preflight, startup
builds the immutable upstream [`passt` commit
386b5f5472b89769c025f5d5056348532a823b93](https://passt.top/passt/commit/?id=386b5f5472b89769c025f5d5056348532a823b93), which contains the
[DHCP broadcast fix](https://passt.top/passt/commit/?id=c0fbc7ef2ae2919bf6162b4149d341f448289836). It diverts the packaged generic and
AVX2 ELF files to root-owned `.distrib` paths and installs both fixed binaries
at the existing `/usr/bin/passt` paths, preserving the profile attachment.
The disposable host also overlays the packaged profile with the
path-qualified `attach_disconnected.path=/tmp/hephaestus-libkrun` flag and
`audit`, while retaining its rules and the narrow
`owner /tmp/hephaestus-libkrun/** rw` rule. `passt` binds its control socket
before pivoting into its sandbox, so AppArmor otherwise sees that socket as a
disconnected path during `accept4`. This flag is limited to the ephemeral
profile; AppArmor documents its path-aliasing risk, so it is not a general
host policy change.

The same disposable-host bootstrap applies a compatibility patch to the pinned
libkrun `v1.19.0` DHCP client. GCE gives the VM a `/32` address while its DHCP
gateway is outside that prefix. `passt` advertises the RFC 3442 classless host
route, but this libkrun client does not consume that option and its default
route netlink request is rejected by Linux. The patch adds the gateway's
link-scoped `/32` route before the default route, preserving the existing
default-route path for ordinary subnets. It is applied only after verifying
the immutable libkrun revision
[`9932c4b59d8f891e60c6aba20d22ebb99ceaa8e2`](https://github.com/libkrun/libkrun/tree/9932c4b59d8f891e60c6aba20d22ebb99ceaa8e2/init)
and fails closed if the expected source sites are not unique. The relevant
implementation is [`init/dhcp.c` at libkrun `v1.19.0`](https://github.com/libkrun/libkrun/blob/v1.19.0/init/dhcp.c).

Run `preflight` first from the `main` workflow, then use `smoke` only after the
quota output and startup image have been reviewed. The cloud dispatch skips
the self-hosted Cooking job. Push and pull-request behavior remains unchanged;
pull-request OIDC is intentionally unavailable because the provider trusts
only the immutable `main` workflow reference and `push`/`workflow_dispatch`
events.

The read-only `cache-preflight` mode runs the same regional quota check and
verifies the designated private Cooking cache object without creating a VM. If
the object lookup fails, it prints the bounded provider error and up to 30
visible object names from that bucket to help locate an upload or permission
mistake.

The `diagnostic` mode is a cheap, deliberate-failure check of the cloud
evidence path. It uses one `e2-small` VM with a 20 GB auto-deleting disk, no
nested KVM, no cache and no Cooking build. Startup checks out the exact SHA,
writes synthetic serial, runtime-state, lineage and browser-summary records,
and invokes the allowlisted collector. The collector scans every retained file,
caps the compressed bundle at 64 MiB, and uploads it to the private
`hephaestus-508000-cooking-diagnostics` bucket under
`cooking/runs/{run_id}/{attempt}/{sha}.tar.gz`. The synthetic test failure is
reported separately from the diagnostics result; the workflow is successful
only when collection, scans, upload, post-delete download and checksum
verification all pass. This diagnostic gate is proven by [run
34586850977](https://github.com/wimpheling/hephaestus/actions/runs/34586850977)
at commit `c252f0517c147c86d6560c79c403fe7ce6f6d4a4`; it completed in 3m11s,
verified VM absence, and passed authenticated private download and scanning.
An isolated diagnostic source containing known fixture content is quarantined
by the current collector; safe lineage/status evidence can still form a
private partial bundle with an explicit `rejectedSources` classification.
That behavior is proven by [run
34593541194](https://github.com/wimpheling/hephaestus/actions/runs/34593541194)
at commit `ddb0920658417fb2bf6538ed7c066c18d8f742ed`: the exact
`runtime-log` was classified `credential-scan-rejected`, the VM was verified
absent before download, and authenticated private download and scanning
passed. Its object is
`cooking/runs/34593541194/1/ddb0920658417fb2bf6538ed7c066c18d8f742ed.tar.gz`
(1,296 bytes, SHA-256
`5b4f25d8e32533f25a5f88217483b3c8ce84649fdacf6f18aa895f0f328717f5`). The
full `gcp-cooking` trial remains pending live validation.

The latest default-image diagnostic [run 34645473842](https://github.com/wimpheling/hephaestus/actions/runs/34645473842)
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
expected unavailable/missing `evidenceScan` result. The full trial at source
`869dd20` remains pending.

The same `main` manual dispatch also offers `gcp-cooking`. Before creating a
paid VM it checks the private, checksum-pinned cache object
`gs://hephaestus-508000-cooking-cache/cooking/heph-gcp-cooking-cache.tar.zst`.
The reviewed archive SHA-256 is
`0ed20efcc1aa019b79405d1eed626b13d4702019e9ceeba2bdde54e45ae29296`; the VM
rejects any other bytes. The object is now uploaded and has been read
successfully by the CI service account, and the full run verified that
reviewed SHA-256, using the explicit project billing/quota project. A
missing or unreadable object stops the job before VM creation. When present,
the VM uses `n2-standard-8` in the selected `europe-west1` zone (default
`europe-west1-b`), the reviewed
`hephaestus-cooking-runtime` service account with the minimum `storage-rw`
scope needed for the private evidence upload (and cache read), a
150 GB balanced boot disk, nested virtualization, and the same 45-minute
provider-enforced `DELETE` lifetime. The startup script downloads and verifies
the cache, checks out the exact workflow SHA, and runs the complete Cooking
path through the checked-out `scripts/gcp-cooking-run.sh` helper. Its deadline
shares the startup script's 35-minute test budget and leaves five minutes for
collection/upload; it is not reset after bootstrap. Cooking runs as a systemd
oneshot with the remaining absolute deadline and a bounded stop timeout, so
activation and teardown cannot consume the collection reserve. The latest full
trial failed as recorded above; full live-path validation remains open pending
focused evidence-gate investigation. Smoke uses the same runtime service account and
`storage-rw` scope for its private diagnostics and cache access.

The implemented `image-build` mode stays in this same workflow, so the existing
exact WIF workflow restriction needs no change. Dispatch it with:

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=image-build -f gcp_zone=europe-west1-d
```

Its direct SA-less disposable builder will bake the reviewed Rust,
libkrun/libkrunfw, passt/AppArmor, Node and browser dependencies into a
SHA-versioned custom image. The output name is
`hephaestus-runner-<first-32-hex-digits-of-manifest-sha>`; the complete
manifest SHA and current recipe, verifier and startup provenance anchors are
stored in the image description. An older image may be supplied when those
anchors remain compatible, subject to the browser lock and baked
browser executable/version checks at startup.

The optional `runner_image` input is supported by `diagnostic`, `smoke` and
`gcp-cooking`. When it is empty, those modes use the optional repository
variable `vars.GCP_RUNNER_IMAGE`; `use_stock_image=true` takes precedence over
both and explicitly selects the stock image. The default candidate variable
now points to the confirmed
`hephaestus-runner-f285fc2b8157f8053383fc98bcaec83d`; the protected rollback is
`hephaestus-runner-2a7223a74f7b32403ea8f586502b8a3f`. For a selected image or a
future replacement, use the
diagnostic command below as the first validation step; the current default
candidate has already passed the custom diagnostic and smoke checks:

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=diagnostic -f gcp_zone=europe-west1-d \
  -f runner_image=hephaestus-runner-f285fc2b8157f8053383fc98bcaec83d
```

To force the stock image, even when an explicit image or repository default is
present:

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=diagnostic -f gcp_zone=europe-west1-d \
  -f use_stock_image=true
```

The candidate may be used for a later full-mode dispatch only after that
check is accepted:

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=gcp-cooking -f gcp_zone=europe-west1-d \
  -f runner_image=hephaestus-runner-<manifest-prefix>
```

Retirement requires an explicit candidate image and refuses the configured
current and rollback images before verifying the candidate's absence:

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=image-retire -f runner_image=hephaestus-runner-<manifest-prefix>
```

A custom diagnostic image uses a 150 GB disk. The source disk must have
auto-delete disabled, the builder must be stopped and deleted while keeping
that disk, and image creation must complete before the source disk is deleted.
Failure cleanup must remove and verify every builder disk and any failed image;
fresh-workflow cleanup and failed-candidate recovery remain planned acceptance
checks until their focused tests pass. Retain one current image and one
rollback image; custom image storage is billable.

Build run [34608196400](https://github.com/wimpheling/hephaestus/actions/runs/34608196400)
at source commit `aa38a0211d8d86264c337b88e1f0081252f3b9fa` completed and
verified builder VM and source-disk deletion at 14:20:20Z. It produced READY
candidate image `hephaestus-runner-925f650c449e8679825f522d8cef52de`. Its full
manifest SHA is
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
builder VM and source disk deleted. It produced READY image
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
This proves the smoke path; the latest full `gcp-cooking` trial is the failed
run recorded at the top of this document. Full validation remains open pending
focused evidence-gate investigation.
The earlier stock-image smoke run 34525055454 (14m57) is retained only as an
observational comparison.

The GCP Cooking path reports a dedicated `HEPHAESTUS_GCP_COOKING` marker and
uses the same private collector/upload/download path. The GitHub workflow
retains only a small, non-sensitive status manifest; the bundle remains in
GCS and can be downloaded with an authenticated command printed by the job:

```sh
gcloud storage cp gs://hephaestus-508000-cooking-diagnostics/cooking/runs/RUN_ID/ATTEMPT/FULL_SHA.tar.gz ./gcp-diagnostics.tar.gz
```

The bucket lifecycle is one day. The existing `cooking` manual mode and
automatic push and pull-request behavior continue to use the prepared
self-hosted runner. The GitHub diagnostics status artifact includes bounded
triage fields; use the runbook's [`summarize-cooking-diagnostics.py`](../../scripts/summarize-cooking-diagnostics.py)
instructions to correlate a first denial with the latest validated attempt
rows. The private bundle remains canonical and requires authenticated
post-delete download, archive validation and credential scanning.

For `diagnostic` and `gcp-cooking`, root owns
`/var/log/hephaestus/cooking-gate-results.json` and
`/var/log/hephaestus/evidence-scan-status.json`. The gate sidecar contains the
checked-out revision, helper SHA-256, mode, runtime aggregate exit,
startup-observed exit, finalized flag and exactly the workload, evidence-scan
and browser-validation gates. Each gate retains only its closed state, exit
code and reason class. Runtime and startup exit codes can differ when the
outer startup deadline fires; retain both. Unfinished gates finalize as
`unknown`/`unfinished`.

Startup copies the finalized sidecars before collection. The collector and
summarizer reject extra fields, secrets, invalid provenance and transitions;
missing legacy sidecars are explicitly unavailable. Inspect
`.triage.gateResults`, `.triage.evidenceScan` and `.triage.runtimeResults` in
the safe status manifest. Acceptance still requires the failed/pass status,
explicit sidecar copy, VM absence, authenticated post-delete download, archive
validation and credential scan; diagnostics success never converts a failed
Cooking workload into a pass. Raw sidecars and reports remain private, and
the safe artifact follows one-day retention. The current whole-suite plus
sidecar regression validation passed 169 focused tests.

If the Cooking oneshot fails, its helper records bounded `systemctl show`
properties. It does not emit `systemctl status` process trees, whose command
arguments can contain fixture values. This source-level safety change is
reviewed; no new full-run root cause is claimed from it.

An earlier full attempt timed out after its raw serial exposed a fixture
credential. At that historical commit the complete diagnostics bundle failed
closed. VM absence was proved at 11:00:45Z/11:00:47Z, but the later download
failure overwrote the status manifest's verified cleanup state with
`cleanup: unverified`. The current collector omits the unsafe source, records
an allowlisted `rejectedSources` classification, and retains an independently
safe partial private bundle with `collectionStatus: partial`; that diagnostic
pipeline is proven by the accepted diagnostic runs above.

The previous full attempt [34630967578](https://github.com/wimpheling/hephaestus/actions/runs/34630967578)
at source `23bb3d4` failed at 18:28:19Z after approximately 26m36s. VM
absence was verified at 18:28:07.989Z, and private diagnostics upload,
post-delete download and scanning passed. Follow-up no-VM triage
[34633918356](https://github.com/wimpheling/hephaestus/actions/runs/34633918356)
at `f278dd6` passed all eight sources, but the triage failure was a Rust
timestamp issue. The current cause is unknown; expected denial or
caught-confinement markers are not proof of a bug. Later configuration and
browser-capture changes addressed the `test-output` and `browser-summary`
projection gap. PR #17 ([track-caller correction](https://github.com/wimpheling/hephaestus/pull/17))
was separate. The latest full attempt is recorded at the top of this document;
full validation remains open.

The current code also passed a local full Cooking run: 33 golden tests passed
with 1 ignored, six PostgreSQL tests completed in 1.75s, two browser reports
passed with zero failures, and cleanup, same-stream live scanning, and the
whole-tree `check-browser-evidence` scan passed. Logs are retained under
`/tmp/heph-local-cooking-20260911`; the cloud failure was not reproduced
locally.

No-VM triage [run 34635329822](https://github.com/wimpheling/hephaestus/actions/runs/34635329822)
confirms aggregate workload exit `1`. An older `browser-summary` derived the
same exit value, but that does not establish a browser cause. Configuration
revision `a107bcc` now has distinct workload/evidence-scan markers, browser
report origin, and strict Rust test-result projection without false panics;
82 focused tests cover the changes. The four track-caller attributes are
merged in [PR #17](https://github.com/wimpheling/hephaestus/pull/17); this does
not claim that cloud validation is complete.

The first successful post-merge live smoke was [workflow run
34525055454](https://github.com/wimpheling/hephaestus/actions/runs/34525055454)
at commit `a8ee9bd803dbde6aef95fa75c1d63458004dfd30`. It ran for 14m57s in
`europe-west1-d` and completed one real libkrun integration test with zero
failures. That test covered the guest network assertions, the private HTTP
gateway, and runtime/cgroup cleanup; the serial output reported
`HEPHAESTUS_GCP_KVM_SMOKE: PASS`. The VM was independently confirmed absent
after cleanup. The retained [serial artifact](https://github.com/wimpheling/hephaestus/actions/runs/34525055454/artifacts/10171404124)
is the evidence record.

The latest custom-image smoke [run 34610546780](https://github.com/wimpheling/hephaestus/actions/runs/34610546780)
failed after 5m31s at `real-libkrun-smoke`; all prebuilt installers were
skipped, and VM absence was verified. Its final report exposed cold-only
revision variables. Source review found and fixed a deterministic prebuilt
instrumentation defect related to those variables, but the live stderr does
not establish that defect as this run's cause. No rerun has passed yet.

This verifies the disposable KVM smoke path only. The cache gate is now
verified: the private checksum-pinned object is uploaded and readable by CI.
An earlier full `gcp-cooking` attempt was [workflow run
34553598335](https://github.com/wimpheling/hephaestus/actions/runs/34553598335)
at commit `da8fa906`. It reached the Cooking test suite with 32 tests passed,
1 failed and 1 ignored, then failed because Skopeo could not access its
`auth.json` (`Permission denied`). The VM absence was verified during cleanup
on 2026-09-11 at 02:41:35Z/02:41:37Z; the retained [serial artifact](https://github.com/wimpheling/hephaestus/actions/runs/34553598335/artifacts/10182422984)
is the evidence record. That historical run left the full GCP Cooking path
pending after the Skopeo permission issue; the newer timeout and bundle
quarantine status are recorded above.

Configure the repository variable `HEPHAESTUS_COOKING_RUNNER_ENV` with the
absolute path of an operator-maintained shell environment file outside the
checkout. It supplies the reviewed, digest-pinned Python and Rust guest images
and repository OCI builder inputs used by the local fixture. Keep prepared image
inputs outside the checkout so checkout cleanup cannot remove them. This file
contains test infrastructure configuration; no Telegram accounts or production
credentials are needed.

The runner must satisfy [preflight.sh](preflight.sh): a non-root account,
read/write KVM access, rootless Podman, writable cgroup v2 delegation for CPU,
memory, I/O and PIDs, supported libkrun/libkrunfw, passt, and the Rust/musl toolchain.
The cooking fixture permits up to 8 GiB of host cgroup memory per VM for OCI
import and verification overhead; its OCI guest RAM remains bounded separately.
The test account must support the guest UID/GID mapping. The browser phase also
requires Node/npm and the Chromium dependencies used by the Playwright suite.
The current host readiness audit confirms the matching Playwright Chromium,
the x86_64 musl target, and the required `skopeo` and `oras` commands. The
operator profile also points to existing external OCI inputs and its reviewed
environment file. These are runner prerequisites; the workflow does not claim
checkout-local source or dependency caches.
Prepare the reviewed
images using the existing platform/repository image workflows before running
the suite. Missing prerequisites fail the job.

The entry point defaults both `TMPDIR` and `HEPHAESTUS_LIBKRUN_TMP_ROOT` to
`/var/tmp`. Use short, disk-backed paths for these large build and VM trees;
a RAM-backed `/tmp` can create memory pressure during OCI verification.
Explicit overrides must be existing absolute directories without a final
symlink. Retained evidence has its own diagnostics directory.

Each execution retains diagnostics in a run-specific directory under
`RUNNER_TEMP`. The inner deadline is 25 minutes plus cleanup grace; the job has
a 30-minute limit. Runs are serialized and a new commit does not cancel an
active guest execution. A failed execution remains failed even if diagnostic
collection succeeds. Diagnostics are uploaded only after the fixture-credential
scanner succeeds; if an individual raw source is rejected, the safe partial
bundle records `collectionStatus: partial` and its `rejectedSources` reason
without retaining that source. The scanner checks retained files and ZIP
contents; this artifact gate does not replace the scenario's checks of raw
storage and guest execution surfaces.

The local entry point also scans the observed execution stream before display
redaction. A fixture credential in raw, base64 or hexadecimal form makes the
command fail, even though matching bytes are removed from retained output.
Service diagnostics that an inner wrapper already redacted are not evidence
of a clean raw service log.
