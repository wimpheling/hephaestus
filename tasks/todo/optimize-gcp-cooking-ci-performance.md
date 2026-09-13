# Optimize disposable GCP Cooking CI performance

Owner: unassigned

## Outcome

Measure and improve the time and cost of the real Cooking CI path while
preserving its test meaning, security boundary, private evidence handling and
fail-closed cleanup. First replace the unavailable self-hosted path for
reviewed same-repository pull requests with a deliberately controlled
disposable GCP path using the reviewed runner image. Fork execution is
separately reviewed and deferred until its trust and approval policy is ready.
Then use instrumented measurements and one-change experiments to reduce
avoidable work.

The experiment ledger is
[`docs/gcp-cooking-ci-performance-experiments.txt`](../../docs/gcp-cooking-ci-performance-experiments.txt).
It is the only place where a change may be recorded as accepted or rejected.
The ledger currently records a valid cold-GCP baseline and accepted CPU,
host-build-removal and overlap trials; broader optimization work remains in
progress.

Current handoff: the newest valid GCP pair is overlap run #34732597502 at
23m38s versus host-removal control #34730591477 at 26m09s, saving 151s
(9.62%) in one pair; the original 26m32s baseline #34724176879 is historical
context. Overlapping workload spans are informational and non-additive. CPU2 and
host-build removal remain accepted modest single-pair results. Instrumentation
is frozen and live-proven through PR #38 and the accepted runs. The local
memory candidate is held, the corrected ext4 scratch candidate is rejected,
and the actual local libkrun probe is feasibility evidence only; GCP input
identity remains unproven. Cancellation propagation is partial, fork execution
is deferred, MVP-06 remains unchecked, and the broader goal remains open.
Critical correction: at measured candidate `7a6ba94`, production
`scripts/run-libkrun-integration.sh` already adds fixed `heph-agent` UID/GID
10001 records in `prepare_guest_root` before preparing and exporting the
verifier root. The account-only 76.180s to 46.561s local pair corrected a
diagnostic harness omission; it is not production mechanism evidence. Withdraw
the identity-projection and image-change candidate. A complete current-input
production OCI operation then passed locally (builder 35.353s, verifier
84.866s, execute 120.512s) with no wrappers; this is local evidence only and
does not establish GCP savings. Draft PR #43 (`9b39eb489d8a87c8d71db6c6c9bb573e124f1891`)
is now the active authorized trial as run `34738676749`, with terminal
measurement and acceptance pending; no retry or second dispatch is planned.
It was prepared for
the separately reviewed Python test-tree hypothesis. It removes only the
guarded `/opt/python/lib/python3.13/test` directory; local OCI time was
105.346s versus 120.512s, with quality session 18452 and the two-page site
fixture passing. This remains unaccepted until the active run's complete
evidence is validated.

## Working-tree implementation status

The trusted controller and instrumentation foundation are live on `main` and
have a valid cold-GCP baseline. PR #38's producer-to-startup-to-collector-to-
controller-to-summarizer lifecycle was reproduced locally with the real timing
scripts, and its required quality gate passed. The instrumentation scope is
now frozen.

The controller accepts an operator-supplied PR number, repository ID
`1312377552`, and exact head SHA only as an all-or-none tuple. It validates the
open same-repository PR through the GitHub API before VM creation. Startup
stages hash-anchored trusted helpers, fetches only the validated public SHA,
and keeps root-owned collection separate from the `forge` PR workload. PR mode
requires the verified runner image with browser dependencies preinstalled;
stock-image PR mode fails closed because its dependency installer would run
PR-controlled package code as root. The cache is frozen read-only, raw
diagnostics stay private, and both PR npm setup and the workload run in the
same systemd sandbox. The sandbox exposes only private per-run state, the
read-only baked browser and Rust toolchain, a writable cargo cache, and the
checkout/evidence paths; owner-based IPv4/IPv6 metadata guards cover the PR
setup, passt and nested guest. Fork execution remains deferred.

Historical diagnostics keep the trusted controller workflow SHA separate from
an optional PR workload SHA: the controller SHA authenticates the selected
`main` Actions run, while the workload SHA locates the private object and
retained workload provenance.

The phase timing helper emits bounded monotonic JSONL records with fixed phase,
trust, provenance, cache and metric fields. Trusted startup projects the
private file into safe diagnostics; workload timings are informational and
never determine correctness. The measured results and evidence status are
recorded in the handoff below; the earlier host-removal run `34727051946`
remains failed/inconclusive and is not a performance measurement.

Measured experiment handoff: the valid baseline is run
[`34724176879`](https://github.com/wimpheling/hephaestus/actions/runs/34724176879)
with the locked configuration and a 26m32s job. The accepted CPU2 experiment
is run
[`34725585641`](https://github.com/wimpheling/hephaestus/actions/runs/34725585641),
which completed in 26m07s, saving 25s (1.57%) in one pair; its repeatability
is unconfirmed. The fresh CPU2 plus host-build control is run
[`34729302087`](https://github.com/wimpheling/hephaestus/actions/runs/34729302087)
with a 26m23s job. The host-build-removal candidate is run
[`34730591477`](https://github.com/wimpheling/hephaestus/actions/runs/34730591477);
it completed in 26m09s, saving 14s (0.88%) against the fresh control. Trusted
controller time decreased from 1552264ms to 1542869ms, a 9395ms (0.61%)
reduction. The host candidate had 39 matching retained markers, browser
validation 2/2 passed, all required gates and 11/11 evidence sources passed,
and verified VM absence. Both earlier candidate results are modest
single-pair measurements with limited repeatability evidence. The accepted
overlap candidate is run
[`34732597502`](https://github.com/wimpheling/hephaestus/actions/runs/34732597502),
which completed in 23m38s (`1418s`) against the host-removal baseline's
26m09s (`1569s`), saving 151s (9.62%). Trusted controller time decreased
from 1542869ms to 1394357ms, saving 148512ms (9.63%); `golden-tests`
decreased 141430ms (12.76%), while `production-project-build` increased
8514ms and OCI builder plus verifier increased 11749ms. The candidate
retained 41 distinct passed marker names: the baseline's 39 plus the two new
preparation regression markers, with no baseline marker lost or failed marker
retained. All workload, browser, timing, evidence, upload/download, scanner,
gate and verified VM-absence requirements passed with 11/11 sources and
browser validation 2/2. The private archive SHA-256 is
`82a4d45146ad35af7974b99a54a3a7303f81293948740ced37f78a99d0c29dc9`. Passing
workload gates and passing diagnostic collection are reported separately from
valid performance measurements; all workload spans remain informational and
non-additive. The single overlap pair is material but does not establish
repeatability or a drastic improvement guarantee.

The CPU implementation was merged through PR #37 (merge
`c0bd1c61f85a6537feb3ee682d178fe92f4ad432`), and the host-build-removal
implementation was merged through PR #39 (merge
`39b8032796637c6898b41c281754c38bb3be41ee`). The integrated candidate quality
gate passed, and the ledger handoff was pushed at `069c8d6`. PR #38 froze the
instrumentation after its real lifecycle regression and quality checks; PR #40
supplied the shared wait fix. The larger optimization sequence remains open,
and the partial GitHub hard-cancellation evidence remains an explicit gap.
The overlap implementation was merged through PR #42 (merge
`6e812d2e0e22c489541eb34fe8d85ce481970d81`) after the full quality gate
(`48642`), independent Astra review, real callsite fixture and valid GCP
comparison. Its next source baseline is the merged main revision; no repeat
run has been claimed.

Post-PR #42 local verifier investigation did not produce another accepted
optimization or a GCP cause. Under the pinned verifier image and a 2 GiB
explicit tmpfs `/tmp`, Syft ran 259.208s and exited 137, showing severe local
tmpfs/reclaim failure behavior; a 4 GiB tmpfs probe passed in 11.784s but is nonrepresentative because it changes storage and
memory, while GCP `/tmp` is disk-backed virtiofs and the host `/tmp` was also
tmpfs. The faithful all-btrfs 2 GiB/2 CPU probe kept the image, script and
197.3 MB candidate unchanged and passed in 14.131s (copy/stage 1.691s, Syft
5.811s, Trivy 2.016s, umoci 4.098s), with 277 package identities and zero
vulnerabilities matching the 4 GiB output. It recorded no OOM or reclaim kill;
the result is not evidence of a GCP memory cause or speedup ratio. Evidence is
at `/home/a/.cache/heph-verifier-faithful-storage-q8lnwgb6/summary.json` and
`/tmp/heph-verifier-local-hjdu2roi/summary.json` plus
`/tmp/heph-verifier-local-hjdu2roi/counterprobe-audit.json`. The remaining question is whether the actual
libkrun verifier path at roughly 364s in GCP corresponds to the local 14s
container path; the current input identity and virtiofs behavior are not
proven locally. The smallest next step is a local actual-libkrun feasibility
probe under the unchanged 2 GiB envelope, separately owned by Luna. The goal
remains open; no cloud dispatch or new candidate is justified by this probe.

The actual local production verifier feasibility probe then passed in 259.460s
under the unchanged 2 vCPU/2048 MiB guest, 8 GiB host cgroup cap, network-off,
UID 10001 configuration, with guest `/tmp` on `fuseblk`. Its unchanged
verifier VM specification, image, script and candidate were used; the
disposable VM and cgroups were cleaned successfully. Copy took 7.285s, Syft
22.789s, Trivy 3.606s and Umoci 225.077s (86.75%), observing a large Umoci
stage cost on that local guest filesystem without establishing virtiofs
causality. The same
candidate in the all-btrfs Podman probe used Umoci in 4.098s and completed in
14.131s. Outputs matched: 277 package identities, zero vulnerabilities and
the expected rootfs manifest. Host peak was 4.260 GB of 8 GiB, with no OOM,
max, PSI or quota signal; rootfs census was 7,739 files, 1,297 directories,
827 symlinks and 488 MB. Evidence is at
`/home/a/.cache/heph-real-verifier-wjf4cisl/summary.json` with exact
provenance and commands in its `provenance.json`, `run-command.json` and
`extracted-spec.rs` siblings.

This is a local feasibility result, not a GCP performance baseline: the GCP
input identity remains unproven, so no 221-second GCP saving can be claimed
from the local 259.460-second verifier result or its comparison with the
14.131-second Podman path. Earlier local Umoci timings near 225s and 61s are
likewise harness-limited by omitted production identity preparation and
unproven input equivalence. The memory candidate remains held and no GCP
dispatch is justified yet. The complete current-input production OCI operation
has now passed locally with builder `35.353s`, verifier `84.866s` and execute
`120.512s`; no immediate GCP, image, cache or bootstrap promotion is justified,
and instrumentation remains frozen.

The corrected local ext4 scratch probe is rejected as an optimization. It
passed correctness and cleanup, but full VM elapsed time was 81.388s (81.474s
including setup/VM cleanup), 4.245s slower than the recent valid virtiofs
unused-disk control at 77.143s. Its Umoci stage was 59.261s and final move
5.944s (copy 3.405s, Syft 9.929s, Trivy 2.289s). Because the candidate did
not win, the planned failed-copy fixture was not run; there was no production,
image or GCP change.

The first preseed-link scratch probe was invalid because the actual script
cleanup removed the precreated output/bundle link before output preparation,
so it did not exercise scratch placement and provides no speed attribution.
The valid virtiofs repeat recorded 77.143s, Umoci 61.064s and move 0.008s, but
is a control only. The corrected ext4 result matched all 9,864 reference tree
entries, including content, metadata, xattrs, owners, symlinks and four
hardlink groups; manifest, package identity and vulnerability identity outputs
also matched. VM destruction preceded disk unlink and cgroup/runtime cleanup
passed. Evidence is at
`/home/a/.cache/heph-scratch-corrected-ntgvpiv_/summary.json` and
`/home/a/.cache/heph-scratch-verifier-t3fo13ky/summary.json`.

These local scratch results establish no GCP cause or saving. Observed Umoci
stage cost remains recorded, while CPU-versus-wait interpretation is pending
Luna's separate existing-data analysis. No production, image or cloud change
is justified; the broader goal remains open.

The account-only real-guest probe is superseded as harness-correction-only.
Its private clone added fixed non-login passwd/group entries for UID/GID
10001, changing one profiled local pair from 76.180s to 46.561s (29.619s,
38.9% local), but production `prepare_guest_root` already adds those records.
Its Umoci and mtree timings, identical outputs and 9,864-entry comparison are
retained as historical diagnostic evidence only; cache/order effects remain
uncontrolled. No GCP optimization, production design candidate or image
change follows. The next diagnostic must execute the complete production
`prepare_guest_root` path with the current generated OCI input before any
verifier-cost attribution. Memory remains held, scratch remains rejected, and
instrumentation remains frozen. Evidence is at
`/home/a/.cache/heph-umoci-account-hj5a9l3v/summary.json`.

The current-input production OCI operation used source `7a6ba94`, the current
Dockerfile, vendor Hugo `9eff60e`, Python base `24b78e`, pinned tools, complete
materialization and `prepare_guest_root`, both guest payloads, fixed
`heph-agent` UID/GID `10001`, and no diagnostic wrappers. Its sealed outer
digest was `sha256:116128c0092d61c2439ba500f004a05fb37b6880e7b9b74001903673ac2d486d`
with 310.3 MB compressed layers, 16,770 regular files, 296 packages and zero
vulnerabilities; Hugo hashes matched pinned values. VM/cgroup/scratch cleanup
passed. This is representative local evidence correcting the stale harness
fidelity gap, but exact byte equivalence to the retained GCP candidate is not
available and no GCP saving is claimed. One pre-provision launcher failure due
to missing empty allowlist directories was corrected before the single actual
operation. The next step is Astra's read-only review of toolchain packaging
and test meaning; no candidate is approved. Evidence is at
`/home/a/.cache/heph-current-oci-p_xchkk6/summary.json` and
`/home/a/.cache/heph-current-oci-p_xchkk6/build-provenance.json`.

Prepared pretrial PR #43 has exact head
`9b39eb489d8a87c8d71db6c6c9bb573e124f1891` and base
`6e812d2e0e22c489541eb34fe8d85ce481970d81`. It changes one guarded Dockerfile
RUN to remove only `/opt/python/lib/python3.13/test`, retaining Hugo checksum
and mode, base/Python/site validation, scanner identities and test meaning.
The locked GCP control is run #34732597502 (`1418s`, controller `1394357ms`,
golden `966549ms`, OCI builder `93001ms`, verifier `364207ms`, complete gates,
cleanup and 41 markers). Rollback is reverting the Dockerfile-only commit.
The draft remains pending the separate paid-trial decision; no GCP dispatch is
included.

## Locked constraints

- [x] Keep the existing WIF provider, exact `cooking-e2e.yml@refs/heads/main`
  workflow restriction, repository and branch conditions, pinned actions and
  CI service account. Do not add `pull_request_target`, widen WIF to arbitrary
  pull-request refs, or let a pull request replace the trusted workflow.
- [x] Keep the current protected runner image
  `hephaestus-runner-f285fc2b8157f8053383fc98bcaec83d` until a separately
  reviewed image change is promoted. Keep one disposable VM, the 45-minute
  provider `DELETE` lifetime, the 50-minute job timeout and one-day private
  diagnostics retention.
- [x] Keep expected test counts, browser assertions, golden assertions,
  scanner checks, marker checks and cleanup verification unchanged. Never hide
  a timeout or failed test behind a performance result.
- [x] Keep diagnostics private, bounded and credential-free in uploaded
  projections. Do not add service-account keys, repository secrets to guest
  processes, public buckets or persistent runners.
- [ ] Keep the MVP-06 joint user-plan review unchecked; this performance task
  does not begin MVP-06 implementation.

## Trusted pull-request execution design

The existing paid GCP workflow is a manually dispatched `main` workflow and
there is no registered self-hosted runner. A pull request must therefore be
represented as data selected by a trusted `main` controller, while the
workflow, permissions and cleanup code continue to come from `main`.

- [x] Define an operator-approved `workflow_dispatch` or trusted main-branch
  controller input containing the PR number, repository ID and exact PR head
  SHA. Resolve the PR through the GitHub API, require that the submitted SHA
  equals the PR head SHA, and record the SHA as the immutable workload
  provenance before creating a VM.
- [x] Treat reviewed same-repository pull requests as untrusted workload code
  and support that path first. Fetch only the public source at the validated
  SHA after the trusted workflow starts; never execute a PR workflow file,
  interpolate PR text into shell code, or use `pull_request_target` with a PR
  checkout and secrets. Fork pull requests are declined/deferred until a
  separate trust and approval policy is reviewed.
- [x] Split trusted setup/collection from the workload. The controller and
  pinned startup code may use the existing WIF identity for resource lifecycle,
  cache staging and post-delete evidence handling. The PR workload must run
  without ADC, metadata credentials, service-account scopes or write access to
  the cache and diagnostics buckets.
- [x] Stage the immutable cache through a trusted setup step before handing
  control to PR code, or prove an equivalent read-only artifact path that
  exposes no credential-bearing metadata. Keep diagnostics upload in trusted
  controller/collector code and upload only the validated safe projection.
- [x] Define the guest boundary explicitly: the PR process receives only the
  checked-out source, selected immutable cache inputs and ordinary test
  configuration; it cannot read the controller's WIF token, runtime identity,
  private key, raw diagnostics, workflow command files or unrelated host paths.
- [x] Use a fixed controller-created resource name and labels containing the
  validated PR SHA, run ID and attempt. Require project-wide ownership checks,
  verified absence after deletion and private post-delete download/scan before
  declaring the run complete.
- [x] Make the controller fail closed before VM creation when the PR identity,
  source SHA, image fingerprint, cache checksum, zone, quota or trust
  conditions are missing or mismatched. Do not retry paid failures
  automatically.

## Measurement and instrumentation

- [x] Instrument monotonic, structured phase records before establishing the
  performance baseline. Cover preflight, VM create, startup, cache hit/miss and
  bytes, dependency/setup, OCI builder and verifier, project build, gateway
  readiness, browser phases, golden tests, credential scanning, archive/upload,
  VM deletion, post-delete download and cleanup verification.
- [x] Record phase start/end monotonic durations, outcome, bounded byte/count
  fields, cache identity and source/image/run provenance. Use an explicit
  schema and fixed enum vocabulary; reject malformed or incomplete records.
- [x] Ensure instrumentation has no shell arguments, command lines, source
  payloads, paths outside the approved field classes, credentials or raw
  browser/network errors. Store only the safe status projection in the
  one-day artifact; keep raw diagnostics VM-private and subject to the same
  archive validation.
- [x] Add focused tests for phase ordering, missing/duplicate phases, timeout
  and cancellation, cache hit/miss, oversized values, malformed fields and
  credential-like input. Confirm the original workload exit and test counts
  remain authoritative before using the timings.
- [x] Establish an instrumented cold-GCP baseline on a justified minimum set of
  runs. Keep image fingerprint, machine type, cache generation, zone and
  concurrency fixed; record baseline SHA separately from each candidate SHA
  when code changes. The historical warm self-hosted PR result is not
  comparable to a cold disposable VM baseline.

## Controlled experiment sequence

- [x] Run the instrumented baseline first and copy its measurements to the
  experiment ledger. Keep image, machine type, zone, cache generation and
  concurrency fixed; keep source SHA fixed only when the candidate makes no
  code change, otherwise record baseline SHA and candidate SHA explicitly.
- [ ] Test one change at a time, beginning with OCI verification and cache/setup
  work, then project-build and gateway-readiness work, then any duplicate npm
  or CI setup work. Record an explicit hypothesis and rollback plan before
  each paid trial.
- [x] Include bounded hypotheses for OCI verification, cache hit/setup, project
  builds, gateway wait, duplicate npm/CI work and cancellation/concurrency.
  Reject any idea that weakens assertions, skips a required test, broadens
  credentials or makes cleanup less reliable.
- [x] Use the smallest justified baseline/candidate comparison within the
  budget policy. Add repeats only when observed variance prevents a decision.
  Compare total wall time, each phase, cache bytes/hits, correctness result,
  scan result, cleanup result and any material cost exposure; do not accept a
  faster run with incomplete evidence.
- [x] Commit the implementation and ledger result for each completed CPU and
  host-build-removal trial on its dedicated experiment branch, recording the
  commit hash, before/after measurements, variance, provenance, test evidence
  and rollback instructions. Any future rejected experiment must revert only
  its own changes, commit the rejection reason and measurements in the ledger,
  and never reset unrelated work.

## Acceptance and handoff

- [x] Prove a trusted controller can run a validated reviewed same-repository PR
  SHA without exposing credentials or PR-controlled workflow execution. Record
  fork execution as deferred unless a separate trust and approval policy is
  accepted.
- [ ] Prove the disposable VM, cache staging, private diagnostics, post-delete
  download/scan and ownership cleanup behavior under success, test failure,
  timeout and cancellation. Success, test-failure, timeout and workload-signal
  cleanup evidence exists; GitHub hard-cancellation propagation remains open.
- [x] Prove the instrumented baseline still passes the unchanged Cooking,
  browser, golden, scanner and marker requirements with complete safe evidence.
- [x] Update the experiment ledger and this task with each accepted or rejected
  result, exact run links, source/image revisions and reproducible commands.
- [x] Keep the current paid GCP manual path available until the replacement PR
  path has its own acceptance evidence; do not claim a self-hosted migration
  from plan text alone.

## Non-goals

This task does not change GCP billing, IAM or WIF configuration by itself,
create a cloud resource, enable a new API, add a self-hosted runner, redesign
the Cooking suite, change application behavior, alter test expectations, or
start MVP-06. Any required permission or trust-policy change is a separately
reviewed implementation decision after the security design above is accepted.
