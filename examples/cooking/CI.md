# Cooking E2E runner

The durable GCP configuration, keyless identities, bucket retention, dispatch
gate and failure triage are in the [GCP Cooking CI runbook](../../docs/gcp-cooking-ci.md).
Current `gcp-cooking` live validation is **pending**; the diagnostic evidence
gate and live partial-source proof have passed, while the historical results
below do not claim that the full path is green. The latest full run
[34588821244](https://github.com/wimpheling/hephaestus/actions/runs/34588821244)
at commit `18fff14` timed out with exit `124`; full runs are re-paused.

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
activation and teardown cannot consume the collection reserve. This remains
pending live full-path proof. The smoke mode continues to use no service
account and no scopes.

The planned `image-build` mode stays in this same workflow, so the existing
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
both and explicitly selects the stock image. The default candidate variable is
not yet configured, and custom image selection remains pending live candidate
validation. After reviewing the build output, the next custom-image check is
diagnostic:

```sh
gh workflow run cooking-e2e.yml --repo wimpheling/hephaestus --ref main \
  -f cloud_mode=diagnostic -f gcp_zone=europe-west1-d \
  -f runner_image=hephaestus-runner-925f650c449e8679825f522d8cef52de
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
The candidate is not promoted; real KVM smoke dispatch remains pending, and
the default image variable is still not configured.

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

If the Cooking oneshot fails, its helper records bounded `systemctl show`
properties. It does not emit `systemctl status` process trees, whose command
arguments can contain fixture values. This source-level safety change is
reviewed; no new full-run root cause is claimed from it.

The latest full attempt timed out after its raw serial exposed a fixture
credential. At that historical commit the complete diagnostics bundle failed
closed. VM absence was proved at 11:00:45Z/11:00:47Z, but the later download
failure overwrote the status manifest's verified cleanup state with
`cleanup: unverified`; that status-writing fix is pending. The current
collector omits the unsafe source, records an allowlisted `rejectedSources`
classification, and retains an independently safe partial private bundle with
`collectionStatus: partial`. Its local regression passes; a cheap diagnostic
live run is still required before treating this behavior as proven. No denial
cause or snapshot evidence is claimed from this attempt.

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
