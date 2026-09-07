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

The official GitHub Actions Linux x64 runner v2.337.0 has been checksum-verified
and prepared in a private directory outside the checkout, but it is not
registered. The repository runner-profile variable is not configured and this
branch/workflow has not been pushed, so this remains prepared configuration,
not evidence of a successful CI run. Runner registration and the first complete
CI execution remain acceptance requirements.

The latest repository quality evidence is recorded as `quality04`: 61 rules
plus two migration gates passed, with actual PostgreSQL/NATS services; the
supplementary authorization, update-admission and Phoenix precommit checks also
passed. Fresh-checkout execution and the first actual GitHub CI run remain
pending.

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
