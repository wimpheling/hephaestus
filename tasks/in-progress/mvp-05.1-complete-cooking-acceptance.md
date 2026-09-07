# MVP-05.1: Complete cooking acceptance

Owner: codex

## Outcome

Complete the remaining [cooking acceptance specification](../../examples/cooking/SCENARIO.md)
using the [existing example](../../examples/cooking/README.md). Extend the
verified first-request journey into a reproducible build/install, multi-request,
upgrade/recovery and security E2E proof. Implementation is authorized on
`mvp-05-e2e-ci`; record verification as each acceptance slice completes.

MVP-05 tests Hephaestus capabilities using deterministic model and relay
endpoints and simulated Telegram-style ingress. Real Telegram delivery, accounts,
Bot API tokens and public Internet deployment are excluded, not deferred
completion requirements. The suite must be reproducibly runnable locally and in
CI. The workflow uses a temporary ephemeral `heph-kvm` runner on the current
KVM-capable host. The authoritative current CI outcome is the
[PR #4 checks page](https://github.com/wimpheling/hephaestus/pull/4/checks);
runner IDs and online state are operational details rather than acceptance
prerequisites.

## Scope decision and main-thread handoff, 2026-09-07

The user explicitly moved the exhaustive
[host-daemon crash matrix](../todo/complete-host-daemon-crash-recovery-matrix.md)
and [expanded adversarial isolation matrix](../todo/complete-adversarial-isolation-e2e-matrix.md)
into separate todo tasks. They are no longer MVP-05 completion blockers.
Do not integrate their external drafts as part of finishing this task.

Retain all existing executable guest-crash, denial, isolation, rotation,
revocation, retirement and secret-confinement checks. Known security defects
remain blockers. Finish the current browser journey and all retained assertions,
prove fresh-checkout local execution and an actual KVM CI run, then complete
quality checks, review and evidence reconciliation. Neither deferred task is
being declared implemented or verified. Historical entries below describe the
scope and evidence at the time; this decision governs current acceptance.

## Verified baseline

Commit `c5c7a8b` contains the applications and installed-artifact proof. The
real-stack test executes one request through Caddy, separate gateway and agent
VMs, PostgreSQL/NATS, SQLite, brokered model/relay calls, controlled Git proposal
and approval, authenticated provenance queries and optional real Hugo output.
Application unit tests cover replay/restart, the framed broker protocol and
SQLite migration/rollback. The full quality gate passed after consolidation.

The harness seeds release metadata. It does not prove isolated application
builds, ordinary installation, browser operation or
the entire failure/update matrix. Existing generic subsystem tests are useful
prerequisites but do not close the cooking-specific acceptance requirements.

## Feature assessment and boundaries

| Area | Current assessment |
| --- | --- |
| Core execution, releases, mailboxes, authority, secrets, controlled Git and inspection | Implemented foundations with a passing joined first-request proof. No additional broad core subsystem is currently established as necessary. |
| Build/install and Hugo artifact lifecycle | Existing build/release primitives are reusable. The audit exposed missing gateway installation, immutable configuration and mailbox allocation operations; narrow implementations and supporting regressions now exist. The joined cooking proof is still in progress. The runner needs both Python and Rust image roots, and the blog needs its actual build declaration. Implement demonstrated gaps narrowly. Seed SQL is not a substitute for product installation. |
| Concurrency, updates, recovery and revocation | Existing mechanisms and focused tests; complete cooking integration remains unverified. Tests may expose further platform defects. |
| Deterministic relay | Exercise the actual relay application with deterministic transport to verify Hephaestus outbound capabilities. Real Telegram transport, accounts, deployment and Bot API tokens are outside MVP-05. |
| Browser journey | The generic management suite passes, including new gateway installation and mailbox allocation controls. Cooking-specific automation is in progress, with a dedicated browser attach harness; gateway mailbox-binding UI remains under implementation. A custom chat UI is not required. |

Keep Telegram parsing, family policy, model behavior, SQLite schema and relay
delivery in the example applications. Keep authority, resource ownership,
isolation and exact provenance in Hephaestus. Do not add persistent gateway
service mode, a generic workflow engine, release-owned custom UI hosting or
remote phone-controlled development as prerequisites for this bounded scenario.
The relay runs externally; the existing short-lived gateway contract suffices.

## Remaining work, in execution order

The [E2E matrix](../../examples/cooking/TEST-MATRIX.md) records the required
triggers and observable outcomes. Keep implementation and evidence aligned with it.

- [ ] **0. Establish reproducible execution and CI**
  - [x] Verify runtime prerequisites and repeatable pinned image preparation.
  - [x] Provide one local/CI entry point with bounded timeouts, isolated resources,
    cleanup and redacted retained diagnostics.
  - [ ] Run the existing cooking journey on a CI runner with real libkrun/KVM.
  - [ ] Expand that job with the remaining matrix as implemented; fail on missing
    required capabilities and report actual executed cases.

- [ ] **1. Exercise real builds and installation**
  - [ ] Provide repeatable preparation of separate forge source repositories
    from the example folders, preserving their exact source revisions and
    offline dependencies. Keep example source canonical in this repository.
  - [ ] Push gateway and agent sources through the existing forge/build path,
    run isolated builds, publish immutable releases, and record source/build/
    artifact/policy hashes. Do not directly insert release/build rows for this proof.
  - [ ] Import and configure the releases through supported authorized
    operations; bind distinct gateway/agent principals, mailbox, state, route,
    exact blog ref and the three secret uses. Record the operations and IDs.
    Gateway configuration must create an immutable runtime revision with typed
    parameters and exact inbound secret bindings under current authority.
    Retain previous revisions; never write a tenant secret-version placeholder
    into the canonical source declaration or mutate an old revision in place.
  - [ ] Build the blog with the declared pinned Hugo image and retain the
    immutable HTML artifact through the existing release/artifact workflow.
    Verify authorized retrieval; the temporary host Hugo check is insufficient
    evidence for that workflow. Do not silently introduce a new hosting service.
  - [ ] Publish v1 and a compatible v2 candidate with the actual update hook,
    plus a deliberate rollback candidate, using ordinary release operations.

- [ ] **2. Extend deterministic multi-request operation**
  - [x] Send concurrent Alice/Bob updates through the actual gateway and verify
    both acknowledgements, normalized envelopes and serialized state effects.
  - [ ] Replay ingress and broker deliveries; inspect SQLite, relay ledger and
    Git proposals to distinguish one logical recipe from retried physical calls.
  - [ ] Stop/restart the agent and then the supervisor, submit later work, and
    prove recovery uses persisted state and delivery records alone.
  - [ ] Exercise competing recipe proposals and stale branch heads; preserve
    exact input commits and explicit conflicts rather than silently discarding
    another recipe or widening Git authority.

- [ ] **3. Prove upgrade and recovery through the platform**
  - [ ] Close the v1 run gate during update, accept simultaneous incoming
    events, drain existing work and run the v2 hook under the exclusive lease.
  - [ ] Verify the migrated recipes/schema, activation, unchanged instance/
    mailbox/route identities, and deferred work selecting v2 only on dispatch.
  - [ ] Exercise explicit application rollback and abnormal termination;
    verify the expected runnable versus paused compatibility-unknown states.
  - [ ] Recover through authorized operator/API operations and verify all
    historical runs, revisions, leases, update decisions and results remain
    inspectable. Do not claim host rollback of application-owned state.

- [ ] **4. Verify retained authority and guest-crash coverage**
  - [ ] Run the existing executable adversarial and denied-authority checks
    without weakening assertions. The expanded resource-isolation matrix and
    missing positive controls belong to the linked adversarial follow-up.
  - [ ] Rotate inbound, model and relay credentials and revoke authority
    during active operations; verify bounded denial, later-version selection
    and historical exact-version inspection without retroactive authority.
  - [ ] Run the existing guest-crash cases around SQLite persistence, model/
    relay calls and proposal-ready state, plus rollback and abnormal update
    recovery. Retain exact retry and outcome assertions. Exhaustive host-daemon
    interruption around ingress, dispatch, import, activation and cleanup
    belongs to the linked crash-matrix follow-up.
  - [ ] Tombstone/revoke attachment, release, route, grant and secret resources;
    verify retained authorized history and denial of new unauthorized work.
  - [ ] Scan database/event storage, logs, traces, metrics, guest files/env/
    arguments and browser evidence for secret sentinels. Existing log/outbox
    checks cover only part of the required surfaces.
    Use distinct values for inbound, model, relay and rotated versions so
    endpoint assertions can detect a credential selected from the wrong binding.

- [ ] **5. Exercise deterministic outbound failure handling**
  - [ ] Inject relay failures and lost responses through deterministic transport
    while exercising the real broker and relay application. Assert bounded
    outcomes, durable inspection and the documented retry/recovery behavior.
  - [ ] Distinguish one logical application effect from retried physical calls;
    do not infer exactly-once external delivery from a local idempotency ledger.

- [ ] **6. Browser acceptance and final evidence**
  - [ ] Extend the existing Playwright management journey for cooking
    installation/binding, operation, approval, provenance, denial, update and
    recovery. Reuse current pages and APIs; record concrete missing controls
    before implementing any UI extension.
  - [ ] Capture redacted screenshots, exact source/release/runtime IDs, commands,
    counts, expected failures and recovery decisions in the example docs.
    The existing Playwright configuration retains traces on failure while the
    journey fills secret inputs. Cover retained trace action/network data in
    the sentinel scan; password masking and DOM assertions alone are insufficient.
  - [ ] Update SCENARIO.md checkboxes only with matching verification evidence;
    document any agreed scope change explicitly instead of dropping assertions.
  - [x] Run Rust formatting, workspace all-target/all-feature Clippy,
    workspace all-feature tests and rustdoc, then `cargo dev quality`.
  - [x] Run the explicit real-stack cooking, fault/update, deterministic relay and browser
    scenarios; record optional test gates as executed or skipped, not simply green.
  - [x] Run `mix precommit`, Mélange drift/doctor and OpenFGA compatibility
    checks required by SCENARIO.md, and the complete sentinel scan.
  - [x] Run `git diff --check` for project-authored changes; preserve vendored
    dependency bytes and their Cargo checksums when upstream whitespace differs.

## Completion evidence

Baseline links: [scenario and recorded IDs](../../examples/cooking/README.md),
[application checks](../../examples/cooking/cooking-agent/VERIFICATION.md),
[authorized run inspection](../../docs/run-provenance-inspection.md).

Append evidence by workstream as it completes. Keep this task in progress until
the remaining acceptance requirements are met or explicitly rescoped by the
user. New generic platform omissions discovered while exercising this task
belong here when needed for acceptance; unrelated expansion belongs in separate tasks.

### Deterministic outbound faults and stale approval, 2026-09-06

The expanded real cooking golden test passed in 44.06 seconds. It asserts the
exact normalized Alice/Bob/follow-up event bodies, a rejected stale Bob approval
after Alice advances canonical Git, malformed model output followed by a
successful retry, and a relay ledger commit followed by a lost response and
idempotent retry. Five logical recipes produce six physical calls at each
endpoint; each fault retains one failed and one completed delivery attempt.
The relay retains exactly five ledger rows. Assertions live in
`examples/cooking/tests/scenario.rs` and `inspection.rs`.

The complete wrapper did **not** pass on this run: its subsequent mailbox
PostgreSQL test compilation encountered an incompatible type in the new mailbox
allocation implementation while that file was being edited. The joined scenario
result establishes this slice only; rerun the complete wrapper after integration.
The build/release fixture was still seeded for this verification.

### Mailbox allocation and admission regression, 2026-09-06

Independent execution of `scripts/test-mailbox-postgres-nats.sh` passed all five
tests in 2.29 seconds using the digest-pinned PostgreSQL and NATS images from
the cooking preflight. This includes authorized mailbox allocation, receipt and
idempotency behavior, removed-instance and outsider denial, and the concurrent
stateful admission regression embedded in the durable delivery test. No matching
disposable mailbox service containers remained after the wrapper exited.

### Multi-request and graceful restart slice, 2026-09-06

The real cooking wrapper passed with Python image digest
`sha256:24b78e523e8cf1732243dc8945d5c75145b5e21d23081a90ecc6b591d4bb820e`:
one joined scenario (36.75 seconds), five gateway PostgreSQL regressions,
and verified runtime/cgroup cleanup. The scenario exercised concurrent Alice/Bob,
duplicate ingress, malformed/oversized/unauthorized input, separate proposal
provenance, serialized leases, and a later request after graceful supervisor
restart using persisted Alice context. This is not abnormal-crash or full
build/install evidence. The optional Hugo step was not part of this run.

The mailbox admission regression ran against disposable PostgreSQL/NATS. With
the admission check removed it failed because both independent stateful events
were admitted; with the check present it passed, leaving the busy event's
original command eligible for redelivery without a new failed logical attempt.
The existing test fixture also needed its required Git ref populated.

### Published source and gateway configuration APIs, 2026-09-06

The build-only cooking slice executed two ordinary Git pushes and two real
isolated builds, then versioned and published the gateway and agent releases.
This establishes build/publication only; installation and the subsequent
scenario remain subject to the integrated rerun. The bootstrap environment
changes used for that run are being reduced and independently reverified.

A fresh migration-only PostgreSQL gateway suite passed six tests with the
pinned PostgreSQL image. Installation reads declarations from the published
release's exact Git source. Gateway configuration creates an immutable revision
with schema-checked parameters and authorized exact credential selections;
mailbox grants are not copied. The clean run required no external role grants
or role alterations. Earlier diagnostic runs with manual role changes are not
clean-setup evidence.

### Deferred update admission regression, 2026-09-06

The generated-client golden regression reproduced the old `FailedPrecondition`
response when requesting an update while a persisted normal run was active.
With deferred admission enabled, the request returns an accepted draining
update without a hook run. After cleanup, one hook is admitted. An explicit
retry admits a distinct second hook. These PostgreSQL/NATS assertions do not
establish the real cooking migration, revoked-actor fencing, or every crash
boundary; those joined assertions remain outstanding.

### Initial installation controls, 2026-09-06

Release and instance pages now expose declared-gateway installation and stable
mailbox allocation. Fifteen focused Phoenix state/presentation tests passed,
including stale installation reply rejection, denied installation without
navigation, and published-only installation controls. This is supporting UI
coverage, not a completed browser journey.

Browser trace capture for the secret journey now begins after request-only
secret creation, preserving subsequent binding/operation/update/recovery
steps without recording deliberately entered credentials as action arguments.
The browser wrapper scans retained files, trace ZIP members, and HTML-embedded
ZIP reports on success and failure. Synthetic plain, base64, compressed and
embedded credential cases were rejected; safe content was accepted. The generic
management browser suite then passed all 12 tests in 41.1 seconds, including
mailbox allocation and reuse. Its retained evidence passed the scan across 29
files and trace archives. The imported-instance screenshot was visually reviewed;
the mailbox confirmation is visible and no credentials are displayed. This is
not the cooking-specific browser acceptance journey. The byte scan does not
establish screenshot/video pixel confinement; secret inputs use password
controls and visual evidence still requires inspection.

### Installation review correction, 2026-09-06

A later resource-creation proof reached real build/publication/import/attachment/
mailbox/gateway operations, but its green result temporarily omitted mutation
receipts from ConfigureGateway and CreateMailboxBinding. That workaround was
rejected: receipt-backed committed product events remain part of the API
contract. Installation acceptance requires restoration of receipt lookups, the
missing binding event fix, a mediated-RPC regression, and another joined run.
It does not supersede the earlier clean gateway adapter evidence.

### Authorization model checks, 2026-09-06

`scripts/generate-authz.sh` passed the Mélange migration drift check.
`scripts/check-openfga-model.sh` and `scripts/check-openfga-service.sh` each
passed eight tests and 119 checks. The service-backed run used the script's
digest-pinned disposable OpenFGA container and exited successfully. These checks
support the authorization implementation; the cooking-specific adversarial,
rotation and retirement assertions remain outstanding.

### Supporting application checks, 2026-09-06

The cooking Python suite passed all ten tests when run with the available Hugo
0.165.0 executable explicitly selected through `COOKING_HUGO_BINARY`. The default
invocation passed nine and skipped that optional test. The blog source check and
five browser-evidence scanner tests also passed. The host Hugo assertion checks
rendered recipe HTML; it does not replace the isolated image/build/artifact proof.

### Workspace and Phoenix checks, 2026-09-06

`cargo test --workspace --all-features` passed after the new mailbox tests were
aligned with the existing optional service-test convention. This invocation had
no PostgreSQL, NATS or KVM test environment configured, so it does not establish
those gated integration cases. Their dedicated wrappers supply the required
services. Pinned-container `mix precommit` passed 238 tests with no failures.
The remaining full quality gate and real cooking acceptance runs are still open.

`cargo dev check protobuf` passed deterministic regeneration, Buf formatting and
lint, all 12 descriptor-policy tests, and compatibility checks. The dedicated
Cooking E2E workflow has been added and its YAML and shell blocks parse; it has
not executed on GitHub because the required KVM runner is not registered.

### Reconciler race and current OCI blocker, 2026-09-06

The feature-gated update-admission regression reproduced the original request
failure when the reconciler won hook admission. With the durable fallback, it
passed while asserting the same persisted hook ID, exactly one run, one command,
and one hook-started outbox record. Both default and `test-fixtures` builds
compiled. This is a PostgreSQL-backed race regression, separate from the actual
guest migration and recovery proof.

The current joined build path is blocked during the OCI builder's approved-base
import. A targeted diagnostic recorded a 4 GiB cgroup memory peak and one OOM
kill; changing guest RAM alone did not fix the host cgroup limit. The disposable
fixture's bounded host-memory allowance is being corrected before rerunning the
full scenario. Browser, update, rotation and storage assertions added after that
point remain unexecuted in the joined run.

The vendored Hugo input now matches the official 0.165.0 Linux amd64 archive
checksum, and `examples/cooking/cooking-blog/verify-hugo.sh` passed. The Dockerfile
retains upstream license material in the produced image. Archive verification
does not establish a successful isolated image or HTML artifact build.

### Quality gate and Hugo scanner remediation, 2026-09-06

The complete `cargo dev quality` gate passed, including architecture, Rust,
Phoenix (240 tests) and UI (95 tests). The fresh PostgreSQL/NATS update-admission
wrapper also passed its ordinary admission, reconciler-race and missing-feature
guard branches. These results supersede the earlier pending quality status;
they do not establish the VM or cooking browser acceptance matrix.

With the fixture's host cgroup ceiling set to 8 GiB, the OCI import diagnostic
passed the earlier OOM boundary. Verification then rejected the official Hugo
binary for 11 fixable HIGH/CRITICAL findings. The replacement uses pinned Hugo
source, Go 1.27.1, and a reviewed vendored dependency update, without weakening
scanner policy. Two offline builds produced identical SHA-256
`9eff60e74ba1387eddbfceb9e48028743d046f06f08eb837eea1daeec88c014c`.
The local image scan found zero policy findings. The obsolete binary archive was
removed, and the CI application command passed all ten then-current Python tests
using the exported rebuilt binary. Real OCI and joined execution remain separate
required evidence.

The joined scenario now calls the blog artifact verifier with the resolved source
commit and restarted daemon, scans raw database rows, retained NATS messages,
and extracted SQLite state, and retains cooking browser screenshots and Phoenix
logs behind the credential scanner. Wiring and Clippy checks do not establish
that these later phases have executed successfully.

The retained-message scanner's dedicated disposable, digest-pinned NATS
regression passed: clean retained messages were accepted, all four credential
sentinels were rejected in payloads, header values and subjects, and scanning
succeeded after the poisoned messages were deleted. This checks the scanner's
behavior independently of the still-pending joined scenario. The expanded
application suite passed all 14 tests with the rebuilt Hugo executable and no
skips.

### Joined execution progress and runner corrections, 2026-09-06

The rebuilt Hugo input passed the real OCI build, verification, publication and
materialization path. A subsequent full invocation reached actual gateway and
agent build/publication/installation, then failed before Playwright started:
rootless Podman could not enter its host pause namespace from the golden test's
mapped user namespace. The browser harness now uses a host-side request bridge;
its mapped-namespace dispatch, deadline and cleanup smoke checks pass. This is
bridge evidence, not a completed cooking browser journey.

The fixture now publishes the Hugo toolchain once from a separate repository in
the same project. Blog content commits consequently avoid queuing redundant OCI
builds. Bounded durable queue checks precede deliberate daemon restarts, after a
diagnostic exposed shutdown waiting on unnecessary image work. Both changes
still require the joined rerun; compilation alone does not prove restart recovery.

The mailbox retry runtime catalog preserves the original mailbox input through
retry ancestry without reopening a completed delivery. Its dedicated regression
passed against fresh, digest-pinned PostgreSQL: retry and retry-of-retry payloads,
ordinary non-mailbox retries, and rejection of missing, cyclic or overlong
ancestry. This supports the browser retry control; the joined retry remains a
separate required assertion.

The fifth full local attempt passed real OCI image build and verification at
the 8 GiB fixture limit, including the packaged Hugo checksum check. It then
failed in the initial blog build before installation/browser execution: the
durable quiescence check found one failed build. The run finished in 186.58
seconds (`/tmp/heph-mvp05-full-acceptance-run5/cooking-execution.nVLCAD.log`).
The same rebuilt Hugo executable and exported-site checker passed on the host
with the initial two-page blog. The guest failure still needs its exact build
diagnostic; the host check does not supersede it.

Fast checks now validate the canonical Dockerfile against the production OCI
policy. Browser assertions identify newly created gateway revisions and exact
retry runs, and the final SQLite inspection requires recipes 42–50 with recipe
50 still pending after active relay revocation. These changes have compile,
Clippy and TypeScript evidence; their joined execution remains outstanding.

The sixth attempt retained the blog guest diagnostic: Hugo completed two pages,
then the checker exited 127 because `/usr/bin/env` could not find `python3`.
Inspection of the digest-pinned OCI layers confirmed Python was present. The
build guest clears its environment and does not inherit OCI `PATH` metadata.
A network-disabled, UID-10001 container with read-only source reproduced exit
127 under `env -i`; exporting the reviewed Python path made the same build and
checker pass. `build.sh` now exports that path and retains its checker invocation.

The fixture's registry aliases now require identical image digests. A focused
regression rejects a same-name layout with a different digest; the Hugo base
continues to resolve to the exact pinned Python image. No scanner or provenance
policy was weakened to resolve either failure.

The seventh completed attempt passed the real OCI build and verifier, gateway,
agent and blog builds, and host browser startup. It stopped at the initial OIDC
callback because the browser fixture supplied a mediator secret shorter than
the server's 32-byte minimum. The retained Phoenix log is
`/tmp/heph-mvp05-full-acceptance-run7b/browser.huRiG6/web-service.log`.
This establishes progress through browser startup, not completion of the
cooking browser journey or the later update, retirement and confinement checks.

Run 8 passed OCI production and verification but exposed a readiness race:
the initial blog build failed with `invalid_build_contract` before creating a
guest, while the worker reported `isolated build image is unavailable`.
The fixture had observed the project image's published `ready` state, which
does not establish that the local worker's root filesystem cache is populated.
The failed run took 187.17 seconds; its execution log is under
`/tmp/heph-mvp05-full-acceptance-run8/`. This is an unresolved production
readiness boundary, not another Hugo or Python execution failure.

The readiness correction now checks the exact local image before an execution
claim, retry reset or verification claim. Missing materialization leaves those
operations unchanged and eligible for command redelivery. A fresh pinned
PostgreSQL regression passed all three paths, including zero verification rows
and zero VM provisions while the image was unavailable; after cache population,
the same execution completed once (`/tmp/heph-mvp05-build-readiness.log`).
The daemon also restores its cache from durable materialized roots at startup
and retries failed manifest writes even when no new image job is claimed.
Its 57 library tests passed, including hydration, manifest retry and command
redelivery regressions. The joined restart and browser journey still require
another full run.

The application suite now includes five real child-process termination cases
using a transformed copy of the cooking agent and the actual deterministic
relay application. Each first attempt exits through SIGKILL, and a durable
one-shot marker permits same-event recovery against the same SQLite database.
The cases cover the initial transaction before and after commit, valid model
response before persistence, relay response before local persistence, and
proposal-ready state before process exit. All 19 application tests passed with
the rebuilt Hugo selected and no skips. These are process/SQLite regressions;
the corresponding released-VM and daemon-crash evidence remains outstanding.

Run 9 timed out while awaiting image verification under heavy host memory
pressure. The available observations do not establish a guest OOM. The runner
now defaults its short-lived runtime and scratch directories to disk-backed
`/var/tmp`, preserving explicit overrides. Inactive task-owned image inspection
directories were archived on disk; unrelated processes and containers were
left intact.

Run 10 passed OCI verification and the initial build/update preparation with
those defaults. The real browser completed login, release gateway installation
and mailbox reuse, then gateway configuration was rejected. The page displayed
"Gateway configuration was not accepted" and "Gateway unavailable"; waiting
for a newly created revision consequently timed out. Diagnostics are retained
under `/tmp/heph-mvp05-full-acceptance-run10/`. The run exited 101 after 243.89
seconds, with cleanup confirmed. This remains a failed full invocation; later
authority, update, retirement and confinement phases were not reached.

The extracted SQLite/WAL/SHM and retained NATS scans now check all six canaries
as raw bytes, base64, lowercase hexadecimal and uppercase hexadecimal. The
SQLite boundary/sidecar regression passed (0.71 seconds); a fresh digest-pinned
NATS instance rejected every encoding in payloads, header values and subjects
and accepted the cleaned stream (0.57 seconds). The disposable server was
removed. Golden-target Clippy passed with warnings denied. These scanner
regressions do not replace scanning the completed joined scenario.

The run-10 database diagnostic identified a duplicate configured-revision hash.
Browser installation reselected the original declaration, then configuration
repeated the fixture's earlier command payload under a fresh command key. The
browser fixture now owns its first configuration. The product correction also
supports later reinstallation: configured revision identity includes the fresh
command key, while command replay still uses its stable payload and receipt.
Fresh revisions receive no copied grants. Two pinned PostgreSQL regressions
passed (1.57 and 0.27 seconds), including configure/reinstall/configure, distinct
revisions, empty new bindings, and historical replay without reactivation.
This supports the next joined run; it does not establish browser completion.

The guest crash slice is now wired into ordinary release publication and
ingress. Updates 51–55 use a separate instance and state volume, with exact
SIGKILL/retry histories and nonoverlapping lease assertions. The shared TLS
fixture routes canonical and crash calls by immutable rule ID and retains
separate physical-call counts. The released variant includes only the runtime
confinement scanner and 24 fingerprints, generated outside its source tree.
It scans its actual authority handoff path, secret mount, release, control,
repository, work, state, environment and argv, emitting bounded before/after
records. Thirteen focused host crash/probe tests passed. This integration has
not yet executed in a real VM; its later full-run result must supply that proof.

The database scan now additionally decodes native `vm.log` JSON byte arrays,
preserving overlap within each ordered run and stream. This closes the gap
where row-text inspection could miss integer-encoded log contents. Two focused
regressions passed: all 24 split encodings are rejected without joining unrelated
runs or streams, and malformed byte arrays fail inspection. The joined scan
requires a nonempty decoded log surface; that execution is still pending.

Full invocation 11 passed preflight and reached the first real VM launch at
19:44:14 UTC. The integration process group then remained stopped, with no
guest execution observed. The worker cgroup reported zero OOM events and an
approximately 20 MiB peak. The runner terminated this attempt (status 143) and
confirmed cleanup of its containers, processes and cgroup. Diagnostics remain
under `/tmp/heph-mvp05-full-acceptance-run11/`; the launch/terminal cause is
under investigation. This supplies no new joined guest acceptance evidence.

The run-11 launch used a PTY pipeline. Its timeout-created process group
differed from the terminal foreground group, and the integration shell, cargo,
golden executable and worker all stopped together. The runner now detaches
TTY invocations with a guarded `setsid --fork --wait` re-exec. Shell syntax,
a pre-KVM TTY validation smoke and the browser host bridge smoke passed;
actual guest startup with this correction remains to be rerun.

Stored build logs now receive an ordered text-chunk scan in addition to raw
row inspection, preventing split encodings from escaping between entries.
The shared decoder has a 1 MiB chunk and 256 MiB aggregate bound. Four local
confinement regressions and golden-target Clippy passed after integration;
the dedicated NATS case was ignored in this invocation (its earlier isolated
NATS result remains separate evidence).

### Latest joined-run status, 2026-09-07

Full invocation 50 reported 31 passing tests, one failure, and one ignored
test. Browser retry passed, and the first failure on active update 50 reached
typed relay denial with zero relay calls. Exact run
`ec8d4e0d…`, event `91ce4f16…`, and relay rule `9965b26f…` were denied after
revocation, but no relay audit row was written because live-lease recheck
returns before the typed-denial record. The revocation-audit correction is in
progress and the assertion remains preserved. Evidence is retained in
`/tmp/heph-mvp05-full-acceptance-run50/cooking-failure.FYE7Oz.log` and
`/tmp/heph-mvp05-full-acceptance-run50/cooking-execution.ryYqiW.log`.
This is not a joined acceptance pass.

Full44's UI watch gap is fixed in `AgentInstanceLive` page watches, generated
handlers, and lifecycle handling; two focused ExUnit tests and compilation
pass. Full45's browser phases passed but produced zero retry requests because
the mailbox source did not support Forge retry. The authoritative
`Run.retry_supported` field, database request-existence check, RPC/generated
types, and Phoenix gate are now covered by Rust (15 and 57 tests), generated
checks, and five Phoenix tests. Authenticated Forge retry now uses a distinct
`retry_source_run_id` and mailbox pending approval. Follow-up corrections
covered unique names (F46), accidental blog-manifest overlay (F47), whitespace
assertions (F48), and asynchronous run-request lookup (F49).

The CI workflow remains unpushed and untracked remotely, with no matching
runner or repository runner-profile variable; effective administration
permission is known to be available, but no CI execution exists. A trusted
same-repository draft PR can trigger the `pull_request` workflow. The external
host-barrier and parent-lifecycle drafts remain uncompiled or unintegrated. The
host crash matrix and adversarial positive controls are explicitly deferred;
final quality gates and the revocation-audit correction remain open.

### Latest joined-run status, Full56, 2026-09-07

Full56 reported 31 passing tests, one failure and one ignored test. The failure
is `examples/cooking/tests/scenario.rs:1682`: the first injected fault attempt
was expected to retain one inspectable record but observed zero. The retained
evidence paths are
`/tmp/heph-mvp05-full-acceptance-run56/cooking-failure.9p5VSn.log`,
`/tmp/heph-mvp05-full-acceptance-run56/cooking-execution.2a41JM.log`,
`/tmp/heph-mvp05-full-acceptance-run56/libkrun-failure-677322.log`,
`/tmp/heph-mvp05-full-acceptance-run56/libkrun-postgres-677322.U4xtQY`,
`/tmp/heph-mvp05-full-acceptance-run56/browser.nk3xpV/playwright.log`,
`/tmp/heph-mvp05-full-acceptance-run56/browser.nk3xpV/web-service.log`,
and `/tmp/heph-mvp05-full-acceptance-run56/browser.nk3xpV/web.log`.
Independent investigation of this assertion is in progress, so Full56 is not
a joined acceptance pass. The exact no-runnable-attachment retirement denial
passed in the same run. Since Full50, the real app-role broker-denial context
lookup and migration guard were corrected
(`migrations/0069_brokered_secret_denial_lookup.sql`), unsupported Forge retry
rejection passed four actual-role PostgreSQL tests, outsider gateway permission
checks were fixed, and the browser journey plus positive Forge retry passed.
These targeted results do not replace the failed Full56 result.

### Joined-run status, Full59 (historical), 2026-09-07

Full59 completed the joined run with exit status 0: 32 golden tests passed, one
test remained ignored, and the PostgreSQL slice passed all six tests. The run
retained `scanner=117 tables=7755 rows`, `NATS=5 streams/2486 messages`,
`VM=4108 bytes`, and `build=2742 bytes`; process, cgroup, container and
temporary-resource cleanup was confirmed. The complete wrapper log is
`/tmp/heph-mvp05-full-acceptance-run59.log`; the retained cooking execution
log is `/tmp/heph-mvp05-full-acceptance-run59/cooking-execution.IC4zsQ.log`.

An earlier quality invocation (`quality01`) stopped at the architecture
generated-types golden boundary. The final quality invocation (`quality04`)
then passed 61 rules plus two migration gates: Rust reported 614 passed, 0
failed and 93 ignored across 204 result rows, rustdoc covered 90 files, Phoenix
passed 247 tests, and UI passed 98 tests, with actual PostgreSQL/NATS services.
Supplementary authz (`authz03`), update-admission (`updateadmission01`) and
Phoenix precommit (`precommit01`) checks also passed. This records final
quality evidence; it does not close fresh-checkout or actual CI acceptance.

Fresh-checkout local execution, the actual KVM CI run and the complete evidence
reconciliation remain pending. The v2.337.0 GitHub
Actions runner artifact was checksum-verified and prepared outside the
checkout, but it is unregistered; the repository runner-profile variable is
absent and this branch/workflow has not been pushed. The host crash matrix and
expanded adversarial positive controls remain explicitly deferred to their
linked tasks and are not MVP blockers.

The prior source-recovery incident remains recorded: the private checkpoint is
`/var/tmp/heph-mvp05-source-checkpoint-20260907T074204Z` with archive SHA-256
`81208d615396896fa1f2eed282c0aadc53b70c80f86c6f49098fb16eb100f01a`, and the
recovery audit is `/var/tmp/heph-scenario-recovery-20260907T041833/`. The
deterministic no-real-Telegram scope and `AGENTS.md` methodology remain in
force.

### Current acceptance status, 2026-09-07

The fresh-checkout local run at commit `4f86a30` (`run03`) passed 33 golden
tests with no failures and one ignored test, all six PostgreSQL tests, and the
retained secret scans. Quality05 passed Rust 615 tests, Phoenix 247 tests, UI
98 tests and rustdoc for 90 files against actual PostgreSQL/NATS services.
Supplementary authorization, update-admission and Phoenix precommit checks also
passed.

One real CI execution ran, uploaded diagnostics and checked them. It exposed a
mailbox projection race that is now bounded by a focused correction. Its
generic browser journey passed all 12 tests, but HTML step metadata leaked a
request-only fixture into the report and failed the archive scanner. Native
input automation and a CI-shaped local run of those 12 tests with the 60-file
archive scanner now pass. A second CI validation is tracked on the
[PR #4 checks page](https://github.com/wimpheling/hephaestus/pull/4/checks),
which owns the final CI outcome; no green result is claimed before that page
reports it.
