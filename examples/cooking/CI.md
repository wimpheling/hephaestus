# Cooking E2E runner

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
balanced persistent boot disk, an ephemeral external address, no service
account or scopes, and a provider-enforced 45-minute `DELETE` lifetime. The
disk is configured for automatic deletion. The VM receives only the exact
workflow commit SHA and the checked-in startup script through metadata; no
credentials or GitHub runner registration token is passed to it.

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

The same `main` manual dispatch also offers `gcp-cooking`. Before creating a
paid VM it checks the private, checksum-pinned cache object
`gs://hephaestus-508000-cooking-cache/cooking/heph-gcp-cooking-cache.tar.zst`
using the CI service account and an explicit project billing/quota project. A
missing or unreadable object stops the job before VM creation. When present,
the VM uses `n2-standard-8` in `europe-west1-b`, the reviewed
`hephaestus-cooking-runtime` service account with the `storage-ro` scope, a
150 GB balanced boot disk, nested virtualization, and the same 45-minute
provider-enforced `DELETE` lifetime. The startup script downloads and verifies
the cache, checks out the exact workflow SHA, and runs the complete Cooking
path through the checked-out `scripts/gcp-cooking-run.sh` helper. Its deadline
shares the startup script's original 40-minute budget; it is not reset after
bootstrap. The smoke mode continues to use no service account and no scopes.

The GCP Cooking path retains the serial console artifact and reports a
dedicated `HEPHAESTUS_GCP_COOKING` marker. It does not yet export a browser
diagnostic bundle from the disposable VM; the serial artifact is the retained
cloud evidence. The existing `cooking` manual mode and automatic push and
pull-request behavior continue to use the prepared self-hosted runner.

The first live smoke trial was [workflow run 34475684487](https://github.com/wimpheling/hephaestus/actions/runs/34475684487).
It created the requested VM and reached the host package and account phases,
then failed in the host cgroup/Podman preflight before any libkrun guest boot.
The retained serial log recorded `HEPHAESTUS_GCP_KVM_SMOKE: FAIL` in the
`cgroup-podman` phase. The cleanup step deleted the VM and confirmed it was
absent. No guest or Cooking E2E pass is claimed. The preceding keyless quota
preflight passed in [run 34475541692](https://github.com/wimpheling/hephaestus/actions/runs/34475541692).

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
scanner succeeds, with seven-day retention. The scanner checks retained files
and ZIP contents; this artifact gate does not replace the scenario's checks of
raw storage and guest execution surfaces.

The local entry point also scans the observed execution stream before display
redaction. A fixture credential in raw, base64 or hexadecimal form makes the
command fail, even though matching bytes are removed from retained output.
Service diagnostics that an inner wrapper already redacted are not evidence
of a clean raw service log.
