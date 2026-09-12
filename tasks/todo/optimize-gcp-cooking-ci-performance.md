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
It is the only place where a change may be recorded as accepted or rejected;
its initial accepted and rejected sections are intentionally empty.

## Working-tree implementation status

The current implementation branch contains the controller and instrumentation
foundation, but no live acceptance result. The trusted workflow remains the
`main` workflow pinned by the existing WIF condition, and must be reviewed and
promoted to `main` before it can exercise these changes. Local contract tests,
Python compilation, Bash syntax checks and diff checks pass for the evolving
working tree; this does not prove a GCP run, a PR run, a performance baseline,
or a security boundary under a live systemd/KVM guest.

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

The phase timing helper now emits bounded monotonic JSONL records with fixed
phase, trust, provenance, cache and metric fields. Trusted startup projects
the private file into safe diagnostics; workload timings are informational and
never determine correctness. No measured performance improvement is claimed,
and no paid dispatch is authorized by this status note.

## Locked constraints

- [ ] Keep the existing WIF provider, exact `cooking-e2e.yml@refs/heads/main`
  workflow restriction, repository and branch conditions, pinned actions and
  CI service account. Do not add `pull_request_target`, widen WIF to arbitrary
  pull-request refs, or let a pull request replace the trusted workflow.
- [ ] Keep the current protected runner image
  `hephaestus-runner-f285fc2b8157f8053383fc98bcaec83d` until a separately
  reviewed image change is promoted. Keep one disposable VM, the 45-minute
  provider `DELETE` lifetime, the 50-minute job timeout and one-day private
  diagnostics retention.
- [ ] Keep expected test counts, browser assertions, golden assertions,
  scanner checks, marker checks and cleanup verification unchanged. Never hide
  a timeout or failed test behind a performance result.
- [ ] Keep diagnostics private, bounded and credential-free in uploaded
  projections. Do not add service-account keys, repository secrets to guest
  processes, public buckets or persistent runners.
- [ ] Keep the MVP-06 joint user-plan review unchecked; this performance task
  does not begin MVP-06 implementation.

## Trusted pull-request execution design

The existing paid GCP workflow is a manually dispatched `main` workflow and
there is no registered self-hosted runner. A pull request must therefore be
represented as data selected by a trusted `main` controller, while the
workflow, permissions and cleanup code continue to come from `main`.

- [ ] Define an operator-approved `workflow_dispatch` or trusted main-branch
  controller input containing the PR number, repository ID and exact PR head
  SHA. Resolve the PR through the GitHub API, require that the submitted SHA
  equals the PR head SHA, and record the SHA as the immutable workload
  provenance before creating a VM.
- [ ] Treat reviewed same-repository pull requests as untrusted workload code
  and support that path first. Fetch only the public source at the validated
  SHA after the trusted workflow starts; never execute a PR workflow file,
  interpolate PR text into shell code, or use `pull_request_target` with a PR
  checkout and secrets. Fork pull requests are declined/deferred until a
  separate trust and approval policy is reviewed.
- [ ] Split trusted setup/collection from the workload. The controller and
  pinned startup code may use the existing WIF identity for resource lifecycle,
  cache staging and post-delete evidence handling. The PR workload must run
  without ADC, metadata credentials, service-account scopes or write access to
  the cache and diagnostics buckets.
- [ ] Stage the immutable cache through a trusted setup step before handing
  control to PR code, or prove an equivalent read-only artifact path that
  exposes no credential-bearing metadata. Keep diagnostics upload in trusted
  controller/collector code and upload only the validated safe projection.
- [ ] Define the guest boundary explicitly: the PR process receives only the
  checked-out source, selected immutable cache inputs and ordinary test
  configuration; it cannot read the controller's WIF token, runtime identity,
  private key, raw diagnostics, workflow command files or unrelated host paths.
- [ ] Use a fixed controller-created resource name and labels containing the
  validated PR SHA, run ID and attempt. Require project-wide ownership checks,
  verified absence after deletion and private post-delete download/scan before
  declaring the run complete.
- [ ] Make the controller fail closed before VM creation when the PR identity,
  source SHA, image fingerprint, cache checksum, zone, quota or trust
  conditions are missing or mismatched. Do not retry paid failures
  automatically.

## Measurement and instrumentation

- [ ] Instrument monotonic, structured phase records before establishing the
  performance baseline. Cover preflight, VM create, startup, cache hit/miss and
  bytes, dependency/setup, OCI builder and verifier, project build, gateway
  readiness, browser phases, golden tests, credential scanning, archive/upload,
  VM deletion, post-delete download and cleanup verification.
- [ ] Record phase start/end monotonic durations, outcome, bounded byte/count
  fields, cache identity and source/image/run provenance. Use an explicit
  schema and fixed enum vocabulary; reject malformed or incomplete records.
- [ ] Ensure instrumentation has no shell arguments, command lines, source
  payloads, paths outside the approved field classes, credentials or raw
  browser/network errors. Store only the safe status projection in the
  one-day artifact; keep raw diagnostics VM-private and subject to the same
  archive validation.
- [ ] Add focused tests for phase ordering, missing/duplicate phases, timeout
  and cancellation, cache hit/miss, oversized values, malformed fields and
  credential-like input. Confirm the original workload exit and test counts
  remain authoritative before using the timings.
- [ ] Establish an instrumented cold-GCP baseline on a justified minimum set of
  runs. Keep image fingerprint, machine type, cache generation, zone and
  concurrency fixed; record baseline SHA separately from each candidate SHA
  when code changes. The historical warm self-hosted PR result is not
  comparable to a cold disposable VM baseline.

## Controlled experiment sequence

- [ ] Run the instrumented baseline first and copy its measurements to the
  experiment ledger. Keep image, machine type, zone, cache generation and
  concurrency fixed; keep source SHA fixed only when the candidate makes no
  code change, otherwise record baseline SHA and candidate SHA explicitly.
- [ ] Test one change at a time, beginning with OCI verification and cache/setup
  work, then project-build and gateway-readiness work, then any duplicate npm
  or CI setup work. Record an explicit hypothesis and rollback plan before
  each paid trial.
- [ ] Include bounded hypotheses for OCI verification, cache hit/setup, project
  builds, gateway wait, duplicate npm/CI work and cancellation/concurrency.
  Reject any idea that weakens assertions, skips a required test, broadens
  credentials or makes cleanup less reliable.
- [ ] Use the smallest justified baseline/candidate comparison within the
  budget policy. Add repeats only when observed variance prevents a decision.
  Compare total wall time, each phase, cache bytes/hits, correctness result,
  scan result, cleanup result and any material cost exposure; do not accept a
  faster run with incomplete evidence.
- [ ] For an accepted change, commit the implementation and ledger result on a
  dedicated experiment branch, record the commit hash, and include before/after
  measurements, variance, provenance, test evidence and rollback instructions.
  For a rejected change, revert only that experiment's changes, commit the
  rejection reason and measurements in the ledger, and never reset unrelated
  work.

## Acceptance and handoff

- [ ] Prove a trusted controller can run a validated reviewed same-repository PR
  SHA without exposing credentials or PR-controlled workflow execution. Record
  fork execution as deferred unless a separate trust and approval policy is
  accepted.
- [ ] Prove the disposable VM, cache staging, private diagnostics, post-delete
  download/scan and ownership cleanup behavior under success, test failure,
  timeout and cancellation.
- [ ] Prove the instrumented baseline still passes the unchanged Cooking,
  browser, golden, scanner and marker requirements with complete safe evidence.
- [ ] Update the experiment ledger and this task with each accepted or rejected
  result, exact run links, source/image revisions and reproducible commands.
- [ ] Keep the current paid GCP manual path available until the replacement PR
  path has its own acceptance evidence; do not claim a self-hosted migration
  from plan text alone.

## Non-goals

This task does not change GCP billing, IAM or WIF configuration by itself,
create a cloud resource, enable a new API, add a self-hosted runner, redesign
the Cooking suite, change application behavior, alter test expectations, or
start MVP-06. Any required permission or trust-policy change is a separately
reviewed implementation decision after the security design above is accepted.
