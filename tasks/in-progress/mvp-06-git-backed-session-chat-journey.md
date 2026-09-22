# MVP 06: Git-backed session chat journey

Owner: Astra orchestration / Luna bounded subtasks

## Outcome

Prove a reference Git-backed project session chat journey that supplies
interaction, isolated execution, and reconnect evidence for the agent-led Heph
distribution. A user starts a chat session backed by a repository, uses the
selected release's chat adapter to submit a message, and an ordinary released
agent reads, commits, and pushes its response with a tightly scoped Git
capability. The agent calls its model API through MVP 04 placeholder
substitution and never sees provider credentials.

This is a precursor for the distribution's primary administration UI. It does
not by itself provide the default installation, instance-administration agent,
or complete create/code/run project journey.

The session layout, message semantics, ordering, concurrency, retention, and
history interpretation belong to the repository and released chat agent. The
chat UI is a distribution-layer adapter for that release, not a platform-owned
universal prompt, workflow, form, or session protocol.

## Locked decisions

| Area | Decision |
| --- | --- |
| Session state | Every session has one project repository whose Git history is the durable, forkable conversation record. Its layout and interpretation are release-owned. |
| Installation | Starting a session creates a fresh repository and instance from a published chat release, binds its symbolic `session` repository requirement, and gives the release/agent responsibility for initialization. No rebuild occurs. |
| User input | The selected release's distribution adapter writes user input according to that repository's contract. Hephaestus attributes the authenticated Git receive but does not construct platform-defined message commits. |
| Agent output | The runtime uses normal Git to commit and fast-forward only its bound repository/ref capability. The repository/release defines how that commit affects a turn; its receive does not recursively trigger the same attachment. |
| Protocol | A chat release pins and documents its own versioned session repository protocol, including records, IDs, ordering, branching, retention, content references, and concurrent-writer behavior. |
| UI | Chat is a release-owned project/session interface and reference interaction path for the selected release. The host supplies generic authenticated repository access and attribution; any message rendering, commands, forms, or richer UI remain release/distribution adapters. The primary distribution entry point is specified by the agent-led distribution task. |
| Model | Automated coverage uses a deterministic fake model HTTPS API. A real OpenRouter smoke test is optional and uses MVP 04 destination-bound placeholder substitution. |

## Dependencies

- [`mvp-01.2-replace-controlled-result-publication-with-runtime-git.md`](../done/mvp-01.2-replace-controlled-result-publication-with-runtime-git.md)
- [MVP 04: destination-bound HTTPS egress](../done/mvp-04-brokered-model-and-outbound-capabilities.md)
- [`release-owned-distribution-ui-surfaces.md`](../done/release-owned-distribution-ui-surfaces.md)
  is a blocking dependency. MVP-06 must not introduce its own iframe, static
  serving, managed UI service, browser handoff, or tab mechanism; it consumes
  the completed release-owned distribution surface.

## Plan review gate

MVP-06 implementation is gated on a joint plan review with the user. The review
must happen before implementation starts or resumes and must confirm the
intended scope, dependency order, and acceptance evidence, including its role as
a precursor rather than the final administration UI. The GCP Cooking validation
follow-up is tracked separately; its current status must be considered during
the review rather than assumed to be complete.

- [x] Review this plan jointly with the user and record the agreed scope and
  sequence here. On 2026-09-21 the user authorized MVP-06 on branch
  `feat/mvp-06-git-backed-chat` and its PR, followed by plugging the completed
  journey into the GCP Cooking workflow.
  The agreed sequence is to audit the existing input-to-response transitions,
  define the release-owned session protocol, implement the new-chat adapter and
  reference agent on the completed release-owned surfaces, prove the journey
  with real Git/browser/VM integration and a deterministic fake model, and then
  add the GCP workflow integration.
- [x] Confirm the release-owned UI dependency is ready, or record the agreed
  dependency resolution before beginning MVP-06 implementation. The completed
  [`release-owned-distribution-ui-surfaces.md`](../done/release-owned-distribution-ui-surfaces.md)
  records completion on 2026-09-21, final `cargo dev quality` success, and
  explicit permission for a distribution journey such as MVP-06 to use a
  release-owned chat tab. MVP-06 therefore consumes that surface and does not
  add another UI host, iframe, static-serving path, or tab mechanism.
- [x] Confirm which current GCP Cooking evidence is relevant to the agreed
  MVP-06 sequence; do not use cleanup-only evidence as a completion claim. The
  recovered no-VM [run 34674597133](https://github.com/wimpheling/hephaestus/actions/runs/34674597133)
  satisfies the current GCP workflow acceptance with all three gates and the
  startup supervisor passing, both browser phases passing, complete retained
  collection, and encrypted archive validation. The original full [run
  34673076889](https://github.com/wimpheling/hephaestus/actions/runs/34673076889)
  remains historically red because it predates the summary projection fix;
  it does not establish a workload or browser failure. This evidence clears
  the dependency review but does not satisfy MVP-06's future GCP integration
  or journey acceptance evidence.

## Implementation checklist

- [ ] **1. Audit the existing interactive path**
  - [ ] Trace user input through durable acceptance, isolated execution,
    visible response, and reconnectable history using repository evidence and
    focused experiments.
  - [ ] Classify each transition as supported, awkward to integrate, or
    genuinely missing, and record the smallest required changes for response
    publication, subscriptions, authorization, and recovery.
  - [x] Record ownership of the visible transcript separately from
    agent-owned model context and internal workflow state.

- [x] **2. Define the reference chat release's repository protocol**
  - [x] Document the reference release's session layout, message identity/order,
    user and agent records, correlation IDs, content references, branch/fork
    rules, concurrent-writer behavior, and compatibility/versioning behavior.
  - [x] Define release-owned initialization, participant policy,
    retention/tombstone behavior, and safe repository fork semantics.

- [x] **3. Build the chat distribution flow**
  - [x] Provide explicitly declared and installation-acknowledged repository
    Git access on the host-owned installed UI origin. Revalidate the live child
    session, exact repository target and human grants for each operation; never
    expose a platform credential to release content.
  - [x] Add the authorized “new chat” workflow: create repository, create the
    session instance, bind its repository capability, let the release initialize
    its repository, and open the release's chat route.
  - [x] Add the reference distribution's chat adapter that renders its own
    repository history, writes user input through its own repository contract,
    displays receive state, and shows committed agent responses.
  - [x] Mount that adapter only through the completed release-owned
    distribution UI surface and its declared tab/API bindings.
  - [x] Keep that adapter and any commands/forms out of the Hephaestus core
    workflow model.

- [ ] **4. Build the reference chat release**
  - [ ] Complete the prerequisite exact-run worktree and internal Git remote:
    a dedicated guest-to-host bridge must work with disabled or broker-only
    networking, use the existing guarded Git receive path and keep credentials
    out of arguments, environment values, Git configuration and logs.
  - [x] Persist runtime receive provenance from the immutable authority
    snapshot, suppress the originating attachment and reject downstream
    trigger candidates without explicit execution authority.
  - [x] Build and publish a small ordinary chat-agent release that defines and
    reads its session protocol, calls its model API through MVP 04, and
    commits its response with normal Git.
  - [x] Bind only the session repository/ref/path capability and its declared
    MVP 04 destination-bound egress bindings; prove that the release cannot
    use its source repository, another session, or an undeclared destination.

- [ ] **5. Prove the journey**
  - [x] Prove one standalone first-turn path from native human Git input through
    production build/release and VM execution to the deterministic HTTPS model,
    canonical assistant response, and runtime Git provenance.
  - [ ] Cover session creation, release-owned initialization, first message,
    agent response, subsequent turn, branch/fork, restart/recovery, concurrent
    release-defined writers, and visibility/history in browser and real-Git
    integration tests.
  - [x] Cover denied source/other-repository access, prohibited ref/path
    writes, delete/force-push attempts, expired/revoked Git capability, and
    recursive-trigger suppression.
  - [ ] Run deterministic fake-model coverage and a separate optional real
    OpenRouter placeholder-substitution smoke test without weakening credential
    controls.

- [ ] **6. Integrate the accepted journey with the GCP Cooking workflow**
  - [ ] Add the MVP-06 real-Git/browser/VM acceptance path to the GCP workflow
    after the local deterministic fake-model proof, preserving disposable VM
    cleanup, private diagnostics collection, credential scanning, and the
    existing workflow gate controls.
  - [ ] Capture and review the GCP run's session creation, turns,
    restart/recovery, fork, negative capability, and fake-model egress
    evidence; classify any failure from retained typed evidence rather than
    inferring a workload or browser failure from an outer workflow result.

## Non-goals

This task does not add a platform-owned session protocol, public HTTP ingress,
mailboxes, WebSockets, streaming model output, generic file upload, a universal
workflow engine, or unrestricted custom HTML. Those may become release adapters
over repositories and content capabilities later.

## Verify and document

- [x] Publish the reference release's versioned repository protocol, including
  its retention and fork warning semantics, rather than presenting it as a
  core Hephaestus contract.
- [ ] Run real-Git and browser integration coverage for the journey and
  negative capability cases listed above. Exercise the released agent in its
  VM runtime with the deterministic fake HTTPS model endpoint; keep an
  OpenRouter smoke test separately opt-in and use only placeholder
  substitution.
- [x] Run `cargo fmt --all -- --check`.
- [x] Run `cargo clippy --workspace --all-targets --all-features`.
- [x] Run `cargo test --workspace --all-features`.
- [x] Run `cargo doc --workspace --all-features --no-deps`.
- [x] Run applicable UI checks when the reference distribution adapter changes.
- [x] Before repository handoff, run `git diff --check` and `cargo dev quality`.
- [ ] Record acceptance evidence with the released protocol version and source
  revision, browser and real-Git evidence for creation, turns, restart/recovery
  and fork, allowed and denied capability cases, and fake-model placeholder and
  credential findings, including any explicitly justified exclusions.
- [ ] Record the GCP workflow integration run, retained artifact identifiers,
  and the typed outcome for each MVP-06 acceptance phase.

## Completion evidence

### Initial code audit (2026-09-21)

Implementation is tracked in draft [PR #53](https://github.com/wimpheling/hephaestus/pull/53).
The initial read-only audit found these boundaries; this is not runtime
acceptance evidence:

| Transition | Existing support and remaining work |
| --- | --- |
| Create session | Repository creation, release import, attachment creation and capability revision exist separately. Compose these generic operations in the distribution; do not introduce a core chat/session model. |
| Open released UI | Completed release-owned static/managed surfaces provide authenticated child navigation. The reference chat adapter still needs implementation. |
| Submit input | Git HTTP authenticates human OIDC/PAT pushes and persists receives. Child UI authentication does not itself authorize Git; a narrowly scoped generic repository boundary is still needed. The release must construct its own commits. |
| Execute and publish | Exact-run Git credentials and guarded receives exist. The documented guest worktree/internal remote and durable runtime receive provenance/originating-attachment suppression need completion and direct verification. |
| Reconnect | Generic repository browsing and resumable repository events exist. The release owns transcript interpretation, response correlation and model context; no platform transcript contract is required. |
| GCP acceptance | Existing Cooking orchestration provides disposable VM execution, two browser phases and retained diagnostics. A new run must exercise the actual MVP-06 artifact; prior Cooking evidence cannot establish chat acceptance. |

Primary code evidence: `crates/git-http/src/lib.rs`,
`crates/forge-postgres/src/repository.rs`,
`crates/run-runtime-local/src/lib.rs`,
`crates/workspace-local/src/lib.rs`,
`crates/run-orchestrator/src/orchestrator.rs`, and the repository-browser,
event and instance adapters under `crates/hephaestus-app/src/rpc/`.
The MVP-01.2 record is in `done/` but retains unchecked implementation items;
its location is not evidence that the missing guest/runtime transitions work.
Keep the full journey and its negative cases open until exercised.

The deeper VM audit found no existing internal Git bridge: broker-only vsock
port 19001 carries the secret broker protocol, and private service port 19002
has the opposite direction and cannot carry runtime authority. The selected
implementation adds a dedicated guest-to-host Git bridge without enabling
external network egress. Runtime receive provenance must resolve the immutable
Git snapshot's repository, not assume it equals the triggering attachment's
repository. Runtime-originated downstream triggers must not bypass execution
authorization through an absent human identity.

### Reference protocol and local Git adapter (2026-09-21)

Commit `64dc01a` adds `examples/session-chat/PROTOCOL.md`, validated canonical
records, and a local ordinary-Git adapter. The release owns the manifest,
participant, human/agent record namespaces, initialization, response
correlation, context separation, fork and retention rules. Actor fields remain
presentation claims, distinct from authenticated host receive attribution.

`PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s examples/session-chat -p 'test_*.py' -v`
passed 14 tests, including temporary bare repositories and competing clones
for concurrent human writes, stale-agent rejection/fresh-run retry, and fork
history without inherited remotes. This is local protocol/Git evidence only;
it does not exercise production capabilities, the released VM agent, browser
or model egress. Published release packaging and full acceptance remain open.

### Explicit installed UI repository Git authority (2026-09-21)

Commit `7161538` adds the release declaration, explicit installation
acknowledgement, immutable generation approval and live repository Git verifier.
Existing UIs default to no Git access. The verifier reuses existing child
session/binding authentication and checks the exact repository and current
human read/write grants. The host smart-HTTP request handler remains pending.

The focused configuration parser and digest tests, nine release-service tests,
and checks of release-postgres, release-service, control-plane-postgres and
agent-config passed. On a disposable PostgreSQL 17 container, the actual
`ui_browser_repository_git_authority_is_explicit_and_live` matrix passed
explicit/no opt-in, wrong repository, read-only write denial, approved write,
revoked write grant, stale generation, expired child, revoked parent, disabled
installation, revoked release and revoked grant cases. The test emitted
`REAL_UI_BROWSER_REPOSITORY_GIT_AUTHORITY=1`; the container was removed after
execution. This proves the verifier matrix, not browser Git or full MVP-06.

### Runtime receive provenance validation (2026-09-21)

Commit `d197bd7`: the focused real PostgreSQL receive/replay test and
smart-HTTP test passed.
The latter uses `RuntimeGitCredentialIssuer`, the PostgreSQL credential
repository, encrypted handoff store, `RuntimeGitHttpAuthenticator`, and the
actual `pre-receive` executable. It exercises an allowed runtime push,
wrong-token rejection, denied-path rejection with the canonical ref unchanged,
durable runtime receive provenance, and human sibling-trigger behavior.
These tests still use fixture setup for immutable authority rows and do not
prove released-agent dispatch or guest execution.

Runtime provenance resolves through the immutable session/run/instance/revision
and Git authorization snapshot, independently of the trigger repository. An
origin attachment is optional. Originating triggers are suppressed; sibling
runtime triggers fail closed because current runtime snapshots do not provide
downstream attachment execution/release-use authority. Human trigger checks
are retained. Full released-VM and recovery acceptance remains open.

### Internal runtime Git bridge (2026-09-21)

Commit `e445536` adds the dedicated vsock 19003 mapping, optional typed VM
bridge metadata, guest loopback proxy and packaged credential helper. The
helper reads the protected bootstrap authority document; its token-free
host/path contract is exercised through native Git credential URL parsing.
Proxy tests cover owned-handle cancellation and half-close response delivery.

`cargo test -p vm-libkrun --lib --bins --tests`, `cargo test -p vm-trait`, and
focused all-target/all-feature Clippy passed. With the actual local libkrun
runtime, `bash scripts/run-libkrun-integration.sh` exited zero: both the
existing VM suite and `real_guest_runtime_git_bridge_forwards_disabled_network_http`
passed. The new test uses a disabled-network guest, packaged helper, actual
vsock mapping and host Unix socket; guest response and cleanup checks passed.
Its host peer is deliberately a transport fixture, not the production Git
service. The daemon Unix listener, exact worktree and composed real Git guest
journey remain to be implemented and verified.

### Host runtime Git listener (2026-09-21)

Commit `4ebe878` adds an owner-only Unix HTTP listener sharing the public Git
service and its receive locks. The libkrun configuration receives the effective
socket path before provider construction. Runtime credential admission precedes
the existing production verifier; human credentials are rejected on this path.
Binding rejects live or unsafe socket paths, and cancellation removes only the
original owned socket inode.

`cargo test -p hephaestus-app --lib runtime_git_listener` passed six tests,
including an actual Unix HTTP exchange, stale/live socket behavior and cleanup.
Focused formatting and diff checks passed. This is listener evidence; the
composed guest worktree, production Git service and released-agent journey
remain pending. Full repository checks and fresh GCP acceptance remain open.

### Runtime workspace resolver validation (2026-09-21)

The pending runtime workspace implementation has focused PostgreSQL evidence:
`cargo test -p workspace-postgres --test postgres_git -- --nocapture` passed
two tests against disposable PostgreSQL 17, removed after execution. Coverage
includes immutable snapshot/provenance resolution, capability repository
selection independent of the trigger repository, missing/cross-repository
target rejection, fetch and ref-scope denial, atomic preparing-row/event
classification, and discovery of interrupted preparing rows for recovery.
Existing proposal recovery remains selected through its separate result state.

Checks and all-target/all-feature Clippy passed for workspace-domain,
workspace-postgres and workspace-local; domain/local tests and focused
formatting passed. This evidence covers the resolver and local materialization,
not orchestrator cleanup ordering, guest Git operations or the released chat
journey. Runtime preparation must follow immutable authority snapshot creation;
the guest needs an exact token-free remote and cleanup after VM termination.
Those lifecycle integration changes and their verification are still pending.

### Runtime worktree lifecycle integration (2026-09-21)

Commit `66d7747` connects runtime Git workspace preparation to run orchestration
after immutable authority creation. The guest receives the exact target commit,
complete reachable history, its symbolic branch, `/workspace/git`, and a
token-free `origin` through the internal bridge. Cleanup follows VM destruction;
recovery follows stale guest cleanup. Runtime cleanup requires its durable
classification, protecting ordinary proposal workspaces. Git's ownership
exception is restricted to the managed guest path.

Focused formatting and Clippy passed, as did 13 orchestrator tests, one
workspace-domain test, four workspace-local tests, two VM credential tests,
18 heph-init tests, and two tests against fresh disposable PostgreSQL 17.
These checks cover lifecycle ordering, mount/remote construction, scoped
materialization and persistence; they do not prove the composed guest journey.
Commit `fd10ad7` corrects the runtime-context dependency: the guest's
repository/ref/commit fields now describe the immutable runtime target rather
than inherit the trigger repository's context. The focused real PostgreSQL
regression passed for differing trigger/target repositories and missing-target
failure, including a rerun after restoring normal database trigger behavior.
This resolver fixture does not substitute for production authority issuance.

### Reference agent packaging and batched publication (2026-09-21)

Commit `5b83cdb` packages the ordinary Python reference agent and its manifest.
It opens `/workspace/git`, handles unanswered human records in transcript
order, and publishes the batch of responses and private context in one commit
and push. Each model request contains only the transcript prefix through its
target input. The release declares scoped runtime Git writes and broker-only
model egress; no core chat protocol or direct HTTPS fallback is introduced.

The focused Python suite passed 19 tests. Compilation, shell syntax and actual
manifest parsing passed. Disposable artifact staging and direct execution of
the installed entry point passed with an initialized idle session. The local
socket-pair broker test verifies wire framing and the sanitized response shape;
it does not exercise the production broker, provider substitution or VM model
path. Actual build/install/dispatch and deterministic model acceptance are the
next composed scenario, not evidence supplied by these package tests.

### Installed UI Git transport (2026-09-21)

Commit `06d8aeb` serves the reserved same-origin Git routes using the live
installed-UI child session and exact repository approval. Browser credentials
terminate at that boundary; the shared Git service receives the verified human
identity and performs its normal repository authorization. Successful discovery
returns the verified human actor ID. Migration 0097 provides complete verified
child context for durable UI audit records.

The focused router/unit suite passed six tests, including missing-cookie and
cross-origin denials. With fresh disposable PostgreSQL 17, the shared resource
matrix passed two tests and the live UI Git route test passed with the actual
PostgreSQL Git authorizer. Git CLI clone/fetch and push traversed the production
router; durable receive actor/request ID and the matching full-context audit
record were verified. The push fixture's maintainer grant is local to that
test, preserving shared revocation-test authority. Focused Clippy and formatting
passed. This proves the host route, not the release browser adapter or the
complete browser/VM/model journey.

### First composed acceptance attempt (2026-09-21)

The pending standalone session-chat golden scenario compiles and uses the
production release build/publish helper, instance import, capability revision,
attachment, secret service and authenticated Git input. Its first real libkrun
attempt reached application startup and failed on the fixture's stale expected
migration version (94 versus applied 97). This is a fixture configuration
failure, not evidence of a successful chat turn. The expectation was corrected
in `02450f4`; the full deterministic model/VM/Git path remains unverified.

### Installed UI repository context (2026-09-21)

Commit `1568ce4` adds `GET /_heph/ui-context`. The production route derives the
repository target from the live child session and repository-scoped installation,
checks the active generation and origin, and records the verified audit context.
It returns only the repository ID. Migration 0098 supplies the bounded
application-role projection; the application schema expectation is now 98.

The focused PostgreSQL resource suite passed two tests, including repository
projection and global-installation denial. Two context route tests passed. The
live PostgreSQL-backed UI Git test passed with an actual TCP context request,
Git fetch/push, human receive attribution and audit. Focused application and
release-service/release-postgres Clippy checks passed. These checks do not prove
the packaged browser's Git implementation or the complete model/VM journey;
production-browser acceptance remains open.

### Trusted-shell session setup (2026-09-21)

Commits `7197133` and `d4d6533` add the project-local New session chat flow and
normalize the generated UI Git-access enum in web RPC projections. Before any
mutation, setup checks the selected release's repository-scoped, full-page
static UI descriptor and explicit Git acknowledgement. It composes existing
repository, import, capability, push attachment, brokered model binding,
installation and browser-handoff operations. Failed attempts retain completed
resource IDs in LiveView state and use stable per-step idempotency keys on retry.
This does not yet establish recovery of setup state across a full page reload.

The web `mix precommit` run passed 313 tests and all 19 architecture checks in
the existing Podman environment. Regression coverage includes the actual
protobuf descriptor projection and a failed capability step followed by retry
without repeating repository creation or agent import. The installed UI launch
hook and project navigation are wired. A read-only review confirmed production
RPC field shapes and typed Git-ceiling selection semantics. Actual browser
setup and the composed response journey remain acceptance work.

### Runtime Git authority after secret binding (2026-09-21)

Commit `13a39ae` fixes `BindSecret` revision creation to carry forward typed Git
authority alongside the cloned generic capability binding. Repository scope,
ref/path rules, operations, limits and exact-parent policy remain unchanged;
the new publication binding points at the new revision's capability row.

The focused secret-postgres Clippy checks and formatting passed. A fresh
PostgreSQL regression with `HEPHAESTUS_POSTGRES_TEST_URL` passed the named
`bind_secret_carries_runtime_git_authority_to_new_revision` test. Post-test SQL
confirmed one seeded runtime-Git release agent and two typed binding rows,
proving the test ran rather than returning through its optional-database skip.
The disposable database was dropped. This supersedes an earlier ambiguous
command report with a misspelled environment variable. The composed VM/model
scenario still needs to verify this path in the complete journey.

### Browser package and response refresh (2026-09-21)

Commits `54a8d4d` and `ec51703` package the repository-scoped reference UI and
add automatic correlated-response polling. The UI initializes an empty session
through ordinary human-authorized Git, reads first-parent history, publishes
human records and refreshes committed responses. Git operations are serialized;
polling has an abortable deadline and stops on page teardown. The regenerated
browser bundle is checked in for the network-disabled release builder.

Nineteen local UI tests passed, including protocol parity, real ordinary-Git
history/initialization, failed publication, correlated response, timeout and
in-flight disposal coverage. The ordinary-Git push fixture uses a native Git
test adapter; this does not prove browser HTTP transport. Release staging passed.

The subsequent standalone real-libkrun composed attempt reached the wait for
`result.completed` but timed out after about 125 seconds. Its golden result was
34 passed, one ignored and one failed. Retained diagnostics are being inspected;
an outer diagnostic query targeted the parent database instead of the isolated
test database, so the timeout alone does not identify the failed lifecycle
transition. Full deterministic response and browser acceptance remain open.

Code review subsequently identified an invalid assertion in this attempt:
`result.completed` and `run_results` belong to proposal-workspace publication.
The reference release disables that workspace and publishes directly through
runtime Git. Its acceptance must instead observe the exact input run's durable
`run.succeeded` event and verify the canonical assistant commit against the
runtime-authenticated `git_receives`/`git_ref_updates` provenance. The earlier
timeout cannot establish whether the agent itself succeeded, because that
run's lifecycle evidence was lost during fixture teardown. The corrected
scenario must retain bounded lifecycle diagnostics before teardown on failure.

### Current composed acceptance status (2026-09-21)

The earlier corrected real-libkrun run (`167d60b7...`) reached a real run,
passed the application socket metadata/`vm.ready` and provenance-boundary
checks, and failed before `run.succeeded` with exit 1. Its golden suite ended
with 34 passed, 1 failed and 1 ignored. The runtime-provenance defect from a
preceding attempt is fixed by `b4bd671`; this historical run does not
establish a successful chat turn or assistant commit and remains separate from
the invalid `result.completed` assertion.

The earlier retained diagnostic (`/var/tmp/sessionchat-typed-diagnostic.log`)
is run `b7db6b79-b0b6-4103-9bc4-6ccd0b6e4318`. It reached `vm.ready`, then
cleaned up with outcome `failed`, exit 1, no guest exit signal, and
`safe_agent_failures=["LocalGitError"]`. It remains historical evidence: the
run did not reach `run.succeeded` or prove an assistant commit.

The model-rule setup correction is implemented and pushed in `aff1233` through
the additive `requested_rule_id` RPC field. Its command and RPC unit checks
pass, but the full production composed turn and browser proof remain open.

Commits `b4bd671`, `aff1233`, `7992e83` and `d1318e2` are reflected in the
current evidence: socket metadata and the provenance boundary are fixed and
the latest lint-only change removes UI/documentation warnings. Commit
`f20fb25` now supplies the release interpreter path and initializes the
release Git identity while reporting a typed safe failure. Commit `666cb89`
adds the verified runtime model-rule path and dynamic pinned-origin catalog.
These are implementation changes, not full journey acceptance. A global guest
`PATH` change was rejected and reverted. The interpreter is release-owned; the
full journey remains unverified. Strict workspace Clippy is still pending.

### Earlier request-time diagnosis and GCP promotion boundary (2026-09-21)

The diagnosis10 request-time finding remains the cause record:
`/var/tmp/sessionchat-runtime-git-stage-diagnosis10.log` reached `vm.ready`,
then emitted `heph_git_auth_stage=runtime_admission_missing` before downstream
authentication and ended with `git operation=push reason=auth returncode=128`.
The subsequent terminal rerun was diagnosis11
(`/var/tmp/sessionchat-runtime-git-fixed-diagnosis11.log`, run
`a5ed7c86-6c73-41d0-a7d7-e0a4e465bfe3`). The challenge fix cleared the prior
auth failure and repeatedly reached `runtime_authority_accepted`, but push now
ends with `git operation=push reason=rejected returncode=1`; it still did not
reach `run.succeeded`. The listener now supplies the Basic-auth challenge; no full
journey success is claimed. Earlier `LocalGitError` and provenance results
remain historical evidence.

The premerge GCP trust boundary is defined by the reviewed controller path. The
`gcp-cloud` job in [`cooking-e2e.yml`](../../.github/workflows/cooking-e2e.yml#L89)
runs only from `refs/heads/main` on `workflow_dispatch`, with WIF pinned to the
`main` workflow reference ([`docs/gcp-cooking-ci.md`](../../docs/gcp-cooking-ci.md#reviewed-same-repository-pr-controller)).
The controller validates the operator-approved PR number, fixed repository ID
`1312377552`, and exact head SHA before VM creation
([`gcp-kvm-smoke.sh`](../../scripts/gcp-kvm-smoke.sh#L222)). The VM checks out
that SHA as workload code, while main-revision startup stages hash-anchored
runtime, gate, scanner, browser-summary and timing helpers; the reviewed image
supplies only pinned preinstalled dependencies
([`gcp-kvm-startup.sh`](../../scripts/gcp-kvm-startup.sh#L310)).

The PR SHA therefore selects workload code only. Trusted
`gcp-cooking-run.sh` invokes its `examples/cooking/run.sh` and exports the
validated Cooking selector and typed gates ([`gcp-cooking-run.sh`](../../scripts/gcp-cooking-run.sh#L995)).
The selector implementation described below is present only on the reviewed
branch commits; the trusted `main` controller/workflow/startup path is not yet
promoted, so premerge GCP still cannot select chat through a PR SHA alone.
Selectable chat validation requires that trusted promotion while preserving
exact repository/SHA validation, private diagnostics, cleanup/post-delete
checks and typed gate results. Merging the trusted-controller changes still
requires user authorization; GCP integration remains within the requested
scope.

Until that promotion, the bounded premerge path remains the standalone local
runner with `HEPHAESTUS_APP_SESSION_CHAT_E2E=1`,
`HEPHAESTUS_APP_LIBKRUN_E2E=1` and
`HEPHAESTUS_APP_COOKING_BUILD_PROOF=1`, without
`HEPHAESTUS_APP_COOKING_E2E=1`; the browser extension is selected by
`HEPHAESTUS_APP_SESSION_CHAT_BROWSER_E2E=1`. Those local results do not carry
the GCP typed gate or trust-boundary acceptance.

The focused web rerun passed 7 tests with 0 failures (`/tmp/heph-web-focused-tests-rerun.log`);
the latest pre-commit web run passed 315 tests with 0 failures
(`/tmp/heph-web-precommit-latest.log`).
The requested-rule command and RPC checks each passed one test, recorded in
`/tmp/heph-requested-rule-command-test.log` and
`/tmp/heph-requested-rule-rpc-test.log`.
These focused checks do not establish full released-agent, browser, VM/model,
or GCP journey acceptance.

### GCP session selector implementation status (2026-09-21)

Commits `7852201` and `c61bddb` add the reviewed branch implementation for a
separately selectable `workflow_dispatch` scenario. `cooking_scenario` is an
allowlisted choice (`cooking`, default; or `session-chat`) and is carried from
the trusted workflow through validated smoke/startup metadata into
`gcp-cooking-run.sh` and the shared [`examples/cooking/run.sh`](../../examples/cooking/run.sh).
The existing Cooking path remains the default. The session branch reuses the
OIDC, host-bridge and browser fixture lifecycle, then invokes the standalone
session runner with the session, libkrun, build-proof and browser flags; it
does not set `HEPHAESTUS_APP_COOKING_E2E=1`.

The session workload phase profile requires the emitted phases
`browser-setup`, `runtime-guest-build`, `oci-image-materialization`,
`gateway-services-ready`, `runtime-worker-build`, `gateway-readiness`,
`golden-tests`, and `database-tests`. The browser projector requires the
strict two-turn `session_chat_new` contract: initialization, send, response,
second send, second response, and reconnect, with complete initial browser
evidence. This is a selector and validation implementation record, not a
cloud-run result: the controller/startup changes remain unpromoted to trusted
`main`, no chat cloud run has been dispatched, and full browser/VM/GCP
acceptance remains pending. The existing exact-SHA/repository checks, WIF
boundary, immutable image selection, private evidence, cleanup and typed gates
remain required.

The focused controller, cleanup, sandbox, image, timing and browser-summary
suite passed 54 tests (`/var/tmp/gcp-chat-integration-focused-final2.log`).
Shell syntax and `git diff --check` also passed. These checks validate the
integration contracts; they do not establish a successful cloud workload.

### Verified standalone first-turn acceptance (2026-09-21)

The clean production-hook run is retained at
`/var/tmp/sessionchat-real-hook-clean17.log` (terminal session `18552`, exit
0), reflected in commit `331afed`. It ran the documented standalone flags
`HEPHAESTUS_APP_SESSION_CHAT_E2E=1`,
`HEPHAESTUS_APP_LIBKRUN_E2E=1`, and
`HEPHAESTUS_APP_COOKING_BUILD_PROOF=1`, without
`HEPHAESTUS_APP_COOKING_E2E=1`. The golden suite passed 35 tests with one
ignored and no failures; the PostgreSQL follow-on passed 8 tests with no
failures; and the runner reported daemon golden E2E passed; runtime and cgroup
cleanup verified. Targeted `cargo clippy -p hephaestus-app --test golden
--all-features` also passed.

The run verifies the native human Git input, production build/release and VM
execution, brokered HTTPS fake-model request, and canonical Git response:
exactly one assistant record has the expected agent role, model text,
`in_reply_to` the human record, and a UUID correlation ID; exactly one
`last_response` context entry carries the same model text. PostgreSQL evidence
correlates the runtime receive with the immutable session, run, instance,
attachment, repository, ref, and commit, and confirms no recursive run request
was created. This is one standalone first turn; browser/new-chat, subsequent
turns, restart/recovery, fork, concurrent writers, negative capability cases,
released-VM denial coverage, and GCP integration remain open.

The prior runtime push rejection was caused by the composed fixture installing
an intentional test pre-receive hook that exited 1 for the session-chat
scenario. The runner now builds and exports the production git-http
pre-receive executable for that scenario, while the rejecting fixture hook
remains the default for other scenarios. Temporary receive diagnostics were
removed before this clean run. Merging trusted-controller changes still
requires user authorization; GCP integration remains within the requested
scope.

Commit `48cc1e7` bounds libkrun diagnostics and preserves interrupted cleanup.
Commit `6ed9c2c` adds the focused real PostgreSQL runtime Git denial matrix.
Against a fresh PostgreSQL 17 database, the exact smart-HTTP test passed with
current migration `98`, three runtime sessions and one persisted runtime
receive; the disposable container was removed. The matrix covers wrong
repository, prohibited ref/path, delete/force push, expired/revoked
credentials, canonical-ref preservation and recursive-trigger suppression.
`cargo clippy -p forge-postgres --test smart_http --all-features` passed; logs
are retained at `/tmp/forge-smart-http-runtime-matrix-migrations98-final4.log`
and `/tmp/forge-smart-http-runtime-matrix-clippy-migrations98.log`. These are
focused authority/denial checks, not full released-agent, browser, VM/model,
or GCP journey acceptance.

Content references currently carry validated metadata and are rendered as
metadata; the text-chat path does not dereference blobs or verify their local
presence/hash. Blob dereferencing is outside this bounded text-chat scope and
adds no new MVP-06 acceptance requirement. The session transcript/context
ownership boundary and the incomplete acceptance boxes remain unchanged.

### Test-only released-guest denial probe (2026-09-21)

The released-VM denial probe is now verified in
`/var/tmp/sessionchat-denial-final7-run.log` for run
`ff43a1ef-6c0b-47f1-933e-0dcd62acc650`. The run passed 35 golden tests with one
ignored test, eight PostgreSQL tests, and VM/cgroup cleanup. The result line
`HEPH_SESSION_CHAT_DENIAL_PROBE host=validated checks=9 refs=unchanged
receives=unchanged` records all nine fixed probe checks, including the
authorized model control; the ordinary agent turn is intentional and excluded
from the unchanged-receive comparison. The published build used its source
repository, the runtime target was a distinct session repository, and the
fixture populated a distinct other repository. The local classification suite
contains seven focused Python tests. An absent or empty, real, unmounted
`/workspace/source` base image directory is accepted; a source checkout is
still rejected.

The probe remains test-only and opt-in through
`HEPHAESTUS_APP_SESSION_CHAT_DENIAL_PROBE_E2E=1`. It uses the production
runtime credential/helper and broker contracts, keeps output to fixed check and
status metadata, and does not claim the broader session journey, browser,
restart/recovery, fork, concurrency, expired/revoked capability, or GCP
acceptance.

### Interactive-path ownership and transition audit (2026-09-21)

The release protocol assigns the visible transcript to Git history: the first
parent chain of `refs/heads/main`, followed by UTF-8 path order within each
commit. User and assistant records are rendered after applying release-owned
tombstones ([`examples/session-chat/PROTOCOL.md`](../../examples/session-chat/PROTOCOL.md#ordering-responses-and-concurrent-writers), [`examples/session-chat/ui/src/git-client.js`](../../examples/session-chat/ui/src/git-client.js#L96)).
The agent's model context is separate release-owned state under the context
namespace and is passed to the model but omitted from transcript rendering
([`examples/session-chat/PROTOCOL.md`](../../examples/session-chat/PROTOCOL.md#forks-retention-and-model-context), [`examples/session-chat/agent.py`](../../examples/session-chat/agent.py#L201)).
This is logical ownership, not a confidentiality boundary: ordinary Git
repository readers can still read those context files.

| Transition | Current classification and evidence |
| --- | --- |
| Durable human input → trigger/run | **Supported through the composed trigger.** The clean standalone run accepted native human Git input and reached the exact input run. The release adapter's human append and expected-parent retry are implemented in [`examples/session-chat/ui/src/git-client.js`](../../examples/session-chat/ui/src/git-client.js#L162). |
| Trigger/run → isolated runtime and model | **Supported for one standalone first turn.** The clean run reached production build/release, `vm.ready`, the released agent, and the brokered deterministic HTTPS model path ([`examples/session-chat/agent.py`](../../examples/session-chat/agent.py#L180)); broader lifecycle and browser coverage remain open. |
| Runtime → assistant Git response | **Supported for one standalone first turn.** The run verified the canonical assistant record, model text, `in_reply_to`, correlation UUID, context entry, runtime receive provenance, and recursive-trigger suppression ([`examples/session-chat/PROTOCOL.md`](../../examples/session-chat/PROTOCOL.md#ordering-responses-and-concurrent-writers), [`examples/session-chat/git_adapter.py`](../../examples/session-chat/git_adapter.py#L287)). Subsequent turns, fork, recovery, concurrency, and denial coverage remain open. |
| Assistant Git response → browser refresh/reconnect | **Adapter supported; end-to-end browser evidence missing.** The UI reads first-parent history and polls for the correlated response ([`examples/session-chat/ui/src/response-refresh.js`](../../examples/session-chat/ui/src/response-refresh.js#L47), [`examples/session-chat/ui/src/index.js`](../../examples/session-chat/ui/src/index.js#L42)). Local adapter/UI checks passed, but real browser HTTP and VM response acceptance remain open. |

This audit records the release-owned transcript/context boundary and the
current transition evidence; it does not complete the remaining audit
workstream or any MVP-06 journey acceptance item.

The completed task records the released protocol version and source revision;
browser and real-Git evidence for session creation, turns, restart, and fork;
the reference agent's allowed session-repository push and denied cross-repository
or prohibited-write attempts; and fake-model egress evidence showing a
placeholder in the guest and no real provider token in guest-visible artifacts.
It also records the verification commands, results, and any explicitly
justified test-environment exclusions.

### Browser initialization acceptance attempt (2026-09-21)

Commit `07bf487` strengthens the browser initialization and per-turn
immutable-provenance assertions; focused formatting, checks, and Clippy passed.
The retained run `/var/tmp/sessionchat-browser-final6-run.log` and
`browser.XBJeEO/playwright.log` show authenticated OIDC reaching the new-chat
LiveView, then failing during the initialization stage. The run ended with
34 tests passed, one failed, and one ignored; it did not send a message, reach
the model, or test reconnect. Browser initialization and the full journey
remain unverified.

The final7 diagnostic run confirms that the creation form becomes visible; the
failure occurs later within initialization. Its enum-only page-state artifact
is `/var/tmp/sessionchat-browser-final7/browser.0xg1ql/playwright-results/session-chat-page-state.jsonl`.
The run again ended with 34 tests passed, one failed, and one ignored. This
narrows the investigation but does not establish a completed browser turn.

The final9 enum-only diagnostic narrows initialization failure to after the
`Create and open chat` submission: release/model options and all form controls
were present, but no installed-UI document followed. The exact composition
operation remains unknown; focused web tests pass (18 tests). Evidence is
`/var/tmp/sessionchat-browser-final9/browser.QHitDq/playwright-results/session-chat-page-state.jsonl`.

An opt-in graceful restart fixture is implemented behind
`HEPHAESTUS_APP_SESSION_CHAT_RESTART_E2E=1` with browser mode enabled. It retains
the first two turns, shuts down and restarts the production daemon with the same
configuration and storage, then opens the existing installation in a fresh
browser and requests a third turn. Assertions cover unchanged prior record
blobs, direct Git ancestry, exact model history, persisted installation and
revision, fresh runtime authority, and no recursive run. Formatting, golden-test
compilation, and strict Clippy pass; retained logs are
`/var/tmp/sessionchat-restart-cargo-{fmt2,check2,clippy2}.log`. The recovery path
has not executed successfully. The GCP scenario now selects it and typed
evidence collection requires both browser phases; runtime acceptance remains
pending.

### Latest browser and phase-gate status (2026-09-21)

The retained final11 markers prove that submitting the new-session form reaches
Phoenix, but compatible model-import validation fails before the RPC. The numeric enum projection bug is fixed in `8b9c4c0`; 31 focused web tests
pass, including a protobuf encode/decode regression. The browser path is not
yet runtime verified after that fix.

Projector commit `b2e12c9` requires both initial and recovery phases while
retaining typed partial failures; its 12 focused tests and collector mode-0600
coverage pass. Commit `900c2ee` typechecks the concurrency browser spec, but it
is not yet wired into or run by the host or GCP harness. Recovery timers, the
GCP restart flag, and required phase checks have focused formatting, check,
Clippy, and 29 scenario/timing tests passing, and shared harness support now forwards the recovery fixture and RPC endpoint.
The fake-child bridge contract verifies sequential phases, retained transcript
counts, and invalid phase/runner rejection. Full initial-plus-recovery runtime
acceptance remains pending.

The post-fix final12 run selected restart acceptance but failed during the
initial browser phase (34 tests passed, one failed, one ignored); recovery did
not start. The retained `/var/tmp/sessionchat-browser-final12-run.log` records
the actual failed `browser-initial` timing. Without creation-step diagnostics
this run cannot identify the current failing setup operation. The broader
journey remains unverified.

The safe reporter now maps the concurrency test and its four stages to fixed
IDs. Seven reporter tests pass; this does not supply concurrency runtime
evidence or complete its host/GCP integration.

The final13 diagnostic run verifies that the projected-policy consumer fix
(`8c89a35`) advances creation: all three model-policy checks pass, release
lookup succeeds, and the repository is created. `ImportAgent` then returns
`invalid`; no browser message turn or recovery phase runs. Retained typed
markers are in `/var/tmp/sessionchat-browser-final13/browser.9b2SCC/web-service.log`.

The broader GCP recovery contract suite passes all 78 tests after aligning the
collector's timing vocabulary with `browser-recovery`. Evidence is retained in
`/var/tmp/sessionchat-gcp-recovery-contracts2.log`. This is helper/contract
validation, not cloud acceptance.

### Local negative-capability acceptance audit (2026-09-21)

The released-VM probe proves source/other-repository read and push denial,
prohibited-path rejection, model destination/rule rejection, and a successful
authorized control (`/var/tmp/sessionchat-denial-final7-run.log`). The real
smart-HTTP PostgreSQL matrix proves ref, delete, force-push, expiry, revocation,
and recursive-trigger denial at the production Git boundary.

Review strengthened expiry/revocation cases to modify the permitted
`runtime.txt` path and assert rejection directly at the production runtime
authenticator, eliminating prohibited-path false positives. Deletion now
requires the fixed guarded-hook denial marker. Both real PostgreSQL tests
pass, with no skips, in `/var/tmp/sessionchat-smart-http-pg-denial-final4.log`;
formatting and focused strict Clippy pass. This closes the listed local
negative-capability item. A fresh GCP run must still retain the corresponding
negative evidence; browser/lifecycle acceptance remains open.

Subsequent retained diagnostics identify and resolve two more setup defects:
`bb012d3` validates backend-compatible session names before any mutation, and
`b6577bb` gives the browser fixture actor the same explicit project capability
delegation role as the standalone fixture. Final15 confirms repository
creation, agent import, capability revision, and attachment creation succeed.
Brokered HTTPS rule declaration then returns `unavailable`; UI installation,
message turns, and recovery have not yet run. Evidence is retained in
`/var/tmp/sessionchat-browser-final15-run.log`.

The declaration failure was a durable receipt contract mismatch. Commit
`0c2f153` appends an `agent_secret_binding.changed` event atomically with rule
creation and requests its receipt in the existing `agent_instance` scope.
No new event aggregate or schema migration is required. A real PostgreSQL
regression verifies the distinct command occurrence, actor, exact scope,
binding version increment, related instance/import IDs, committed outbox row,
and same-command replay without duplicate events. It passes without skips in
`/var/tmp/sessionchat-secret-receipt-focused4.log`; focused strict Clippy and
application compilation also pass. Browser acceptance remains pending the
fresh final16 run; this regression alone does not prove the interactive journey.

Final16 confirms `declare-brokered-rule status=ok`, then the page reports a
partial setup error before installation/handoff evidence. The run terminates
with exit 101 (34 passed, one failed, one ignored); neither a browser message
turn nor recovery runs. The next operation in the setup source is installation,
but retained markers do not yet identify the failing boundary. Evidence:
`/var/tmp/sessionchat-browser-final16-run.log` and its retained browser directory.

Commit `5f2f2cd` strengthens the host runtime-Git materialization proof to compare
every reachable object through a three-commit history and assert nonempty,
disjoint source/checkout object-file device/inode sets. The exact focused test
runs one test and passes in `/var/tmp/sessionchat-workspace-history-test.log`;
formatting and strict workspace-local Clippy pass. This closes those two host
evidence gaps; the broader runtime credential-surface check remains open.

The post-declaration failure was UUID text casing: the web generator emitted
uppercase hexadecimal, while Rust returned canonical lowercase text. Commit
`ee54695` fixes the generator and makes the setup fake return Rust-shaped
lowercase IDs. Twelve focused UUID/client/setup-state tests pass, including
the semantic mismatch negative case. Final17 confirms the rule ID matches
and UI installation succeeds, then fails browser-handoff validation. It ends
with 34 passed, one failed, one ignored; browser turns and recovery are still
unverified (`/var/tmp/sessionchat-browser-final17-run.log`).

Commit `cb40745` adds the concurrency selector/phase to the shared browser
bridge. Shell contracts pass for sequential initial/recovery/concurrency
handoffs, including retained transcript/agent counts and both selector/phase
mismatch rejections. This is bridge validation, not concurrency execution.
The stale push is rejected before durable receive persistence: host acceptance
must expect four accepted receives for the two turns and use browser Git
packet/retry evidence for the rejected attempt, not require a fifth audit row.

Commit `7deca86` extends typed session-chat browser evidence to require all three
initial/recovery/concurrency reports. Initial plus recovery alone remains
partial; a failed concurrency phase retains its typed failure. Collector and
timing vocabularies include concurrency. The broader Python GCP contract suite
passes 79 tests (`/var/tmp/sessionchat-gcp-concurrency-contracts.log`); this is
not a cloud run or host concurrency execution.

Commit `512a687` adds a tenth released-guest check for the actual runtime Git
credential. It obtains the helper password only in guest memory, scans actual
Git subprocess arguments, effective environments, captured output, and resolved
configuration, and emits only a fixed result after the authorized agent turn.
The intended helper password channel is excluded. Proxy and CLI regressions
cover contaminated outputs, inherited environments, exceptions, and restoration.
The real released-VM run `/var/tmp/sessionchat-denial-final8-run.log` verifies
all ten checks, unchanged denied-repository refs/receives, the authorized turn,
35 golden tests (one ignored), eight PostgreSQL tests, and runtime/cgroup cleanup.
Formatting, golden compilation, and focused strict Clippy also pass. This
evidence covers the exercised guest Git surfaces, not arbitrary host logging,
process memory, or the intentionally protected authority document.

Commit `b9faca4` fixes final17's handoff projection mismatch by supplying the
browser URL builder's `route_base` from the requested route, matching existing
installed-UI navigation. The state regression invokes the real success callback
with a canonical RPC response and consistent valid installation/generation IDs,
then requires a successful bootstrap URL. Removing the production mapping makes
that test fail with `invalid_projection`. The focused state/client/browser suite
passes 23 tests (`/var/tmp/sessionchat-route-success-focused.log`), with the
negative mutation proof in `/var/tmp/sessionchat-route-success-negative.log`.
The browser rerun remains pending; no turn/recovery acceptance is inferred.

Commits `d927054` and `f791d0c` wire the opt-in concurrency host phase and GCP
selection/timing requirements. Host checks cover two correlated turns, exact
receive/run provenance, four accepted receives, retained ancestry, and unchanged
prior record blobs. Compilation, formatting, and strict golden-test Clippy pass;
runtime concurrency remains unverified. Review also corrected model-payload role
assertions to `user`, matching the released agent (Git actor roles remain human).

Final18 reaches successful server provisioning and handoff but fails installed
UI initialization. Final19 adds fixed, credential-free browser categories and
confirms the expected document returns HTTP 200 with HTML content, the correct
path and `Session chat` title, and the chat root/input present. Initialization
still fails afterward, before any verified turn, recovery, or concurrency phase.
The earlier title-failure inference is disproved. Evidence is retained in
`/var/tmp/sessionchat-browser-final19-run.log` and
`/var/tmp/sessionchat-browser-final19/browser.perlIe/playwright-results/session-chat-safe-diagnostics.jsonl`.

Commit `ce659f1` fixes Playwright discovery of the concurrency spec and adds a
forked-target browser spec. Exact selector listings each discover one test;
TypeScript and nine safe-reporter tests pass. The fork spec is not host-wired or
runtime-verified and does not complete fork acceptance.

Final20 narrows the remaining initialization failure: installed UI context and
Git discovery both return successful responses, context has the expected cache
policy and shape, and discovery contains a canonical verified actor header.
The UI then enters its connection error state. This does not yet identify the
failing Git operation or prove any browser-backed turn. Fixed-category evidence:
`/var/tmp/sessionchat-browser-final20/browser.2kxWYH/playwright-results/session-chat-safe-diagnostics.jsonl`.
The pinned HTTP transport's header normalization matches the client; focused UI
tests pass 19/19 (`/var/tmp/sessionchat-ui-focused-tests.log`), but those tests do
not substitute for the failing installed-browser path.

Commit `4c6803f` requires all four browser phases (initial, recovery,
concurrency, fork) in the GCP session-chat evidence and workload timing gates.
The runner selects the fork flag; collector and final diagnostic summary retain
the canonical phase order. The 133 focused Python contracts cover successful
four-phase projection, missing fork as incomplete, and failed fork as a typed
failure (`/tmp/heph-gcp-fork-contracts-20260921.log`). Shell syntax and diff
checks pass. This is contract validation, not a completed local or cloud journey.

Final21 stopped during compilation of incomplete fork test wiring. After that
wiring compiled, final22 stopped during production build source materialization
with `isolated build source contains an unsupported object` (34 golden tests
passed, one failed, one ignored). Both runs cleaned up without reaching
Playwright; neither produced connection-operation evidence. The final22 run log
is `/var/tmp/sessionchat-browser-final22/cooking-execution.jePnmC.log`.

Final22's unsupported objects were generated `ui/node_modules/.bin` symlinks
copied into the release fixture after an isolated UI dependency install. Removing
only those generated dependencies restored source materialization. Final23 then
reached the installed browser and reproduced the connection error, but its new
diagnostic hook accessed `document.documentElement` before it existed. The hook
raised TypeErrors and did not activate operation capture; this run does not
identify the underlying Git failure. Evidence is under
`/var/tmp/sessionchat-browser-final23/browser.soUUFd/`.

Commit `b7b2456` wires the optional fork host phase after concurrency. It captures
the source's actual model rule/binding, publishes inherited history through the
production Git boundary, creates fresh target authority and UI installation,
and checks the target turn's exact commits, records, receives, run count, and
runtime revision. Source history and records remain invariant in the assertions.
Formatting, golden compilation, and strict golden Clippy pass; logs are
`/var/tmp/session-chat-fork-cargo-{fmt,check,clippy}-final23.log`. Fork browser/VM
execution remains unverified.

The diagnostic hook now uses a global opt-in set before module execution. A
real Chromium probe against the packaged UI verifies the flag, fixed discovery
error capture, and zero page errors (`/var/tmp/sessionchat-ui-init-script-probe.log`).
Final24 then captures the actual failing operation as `discovery`, status
`failed`, code `Error`, despite successful HTTP response and valid actor header.
This localizes the failure to `getRemoteInfo` before initialization, clone, or
history reading; the underlying cause remains unresolved. No browser-backed
turn is proven. Safe sidecar:
`/var/tmp/sessionchat-browser-final24/browser.SAx5AU/playwright-results/session-chat-safe-diagnostics.jsonl`.

The discovery failure is reproduced against a real empty Git HTTP backend:
the browser bundle lacked the `Buffer` global required by isomorphic-git's
stream reader. Adding a direct pinned `buffer` dependency and installing that
global lets Chromium complete discovery, initialization, push, and read. The
permanent smoke also checks the pushed manifest and participants; restoring the
old bundle reproduces `MissingBufferDependency`.

Final25's first launcher lacked the browser export and was interrupted before
browser execution; it is not acceptance evidence. The corrected run under
`/var/tmp/sessionchat-browser-final25-corrected/` enables browser, restart,
concurrency, and fork. Installed-browser initialization and send pass, then the
first response wait times out. Cleanup completes; no recovery, concurrency, or
fork acceptance is inferred. Investigation now follows the first response path.

Commits `9e6cd6e`, `41ca4a9`, `957ba52`, and `7cfd1e7` add an opt-in separate
negative guest process and its typed evidence pipeline. The process selects one
exact golden test, removes browser and nested timing flags, and keeps its log
private. Success requires that test to pass and exactly one ten-check host
validation marker. The projector is tested against real libtest output; runner
contracts pass 36 tests, and collector/triage checks pass 93. GCP enablement and
fresh combined runtime evidence remain pending.

Commit `6fadc0b` contains the focused Buffer fix and packaged Chromium/Git smoke;
temporary client and server diagnostic instrumentation has been removed.
The corrected final25 execution log records the initialization receive, human
receive, a runtime-principal receive, and successful worker exit/cleanup
(`cooking-execution.vR9DpQ.log:446-465`). The agent publishes only after its model
call, so this narrows the remaining failure to browser refresh/display rather
than an absent runtime publication. It does not replace the canonical host turn
assertions, which the failed browser phase prevented from running.

Commit `af87743` enables the separate negative process for GCP session-chat and
requires both `guest-negative-capability` timing and the strict typed summary.
The final gate uses the collector's shared validator after cleanup/download;
missing, failed, duplicate, and incorrectly typed evidence is rejected. The
combined negative/GCP contract suites pass 162 tests
(`/tmp/heph-session-negative-crossstage-20260921.log`). No cloud acceptance is
claimed.

Workspace formatting and `cargo clippy --workspace --all-targets --all-features`
pass (`/var/tmp/hephaestus-cargo-{fmt-check,clippy-workspace-all}-gcp-sessionchat.log`).
The first workspace test run stopped because `HEPHAESTUS_POSTGRES_TEST_URL` was
unset, after 1,195 passed tests, one failure, and five ignored tests. It is not
a passing workspace gate; a rerun with a disposable PostgreSQL fixture is pending.

Commit `809ae39` fixes the next browser failure: the response controller stored
unbound browser timer functions and invoked them with the controller as receiver,
causing `Illegal invocation` after the human push. Receiver-safe wrappers fix it.
The packaged Chromium smoke now publishes a human record, accepts a native Git
assistant commit, fetches/reads it, and displays both correlated records. The
old Buffer-fixed bundle fails with `Illegal invocation`; the new bundle passes
(`/var/tmp/sessionchat-refresh-{old,new}-bundle-smoke.log`). All 20 UI tests
pass from an external dependency tree, and two generated bundle builds match
byte-for-byte. A fresh full installed-browser/VM run remains required.

The PostgreSQL-backed workspace rerun passes: 1,196 tests passed, none failed,
95 ignored across 236 result groups
(`/var/tmp/hephaestus-cargo-test-workspace-all-pg-gcp-sessionchat.log`). The
previously failing runtime Git resolver test passes against that disposable
database. Ignored cases include NATS-dependent integration, UI installation
PG/NATS composition, Zot/Podman, publication guards, daemon TLS composition,
and generated Connect doctests; this is not evidence for those paths.
`cargo doc --workspace --all-features --no-deps` also passes
(`/var/tmp/hephaestus-cargo-doc-workspace-all-gcp-sessionchat.log`). The four
required Rust baseline commands are complete; the repository-wide quality gate
and full runtime/cloud acceptance remain pending.

Final26 passes the initial installed-browser journey and canonical host
validation for two turns, then captures restart state. The evidence is
`/var/tmp/sessionchat-browser-final26/cooking-execution.OpuIor.log:478-480`.
This verifies the authorized new-chat composition, release-owned Git adapter,
and installed UI surface for the initial flow. Recovery fails before launching
the installed UI: its test looked for a repository-scoped installation on the
project page. Concurrency, fork, and negative phases were not reached. The
remaining launch-path correction and a fresh full run are required before
claiming recovery or overall journey acceptance.

The versioned protocol now publishes explicit release-owned fork and tombstone
warning semantics: forks retain reachable history and require fresh target
authority/model binding; tombstones hide ordinary views without erasing Git
history. These obligations do not introduce a platform approval flow.

The full `cargo dev quality` attempt with disposable PostgreSQL/NATS and both
isolated RPC proofs enabled stops at architecture validation, before later
gates. Diagnostics are `DB-STATIC-SQL` in the UI browser schema test and
`RPC-NON_RPC-HTTP-ALLOWLIST` for the dedicated runtime Git listener and
installed-UI repository Git routes. Log:
`/var/tmp/hephaestus-cargo-dev-quality-gcp-sessionchat.log`. Both disposable
services were cleaned up. These diagnostics must be resolved before the
repository-wide quality gate can be marked complete.

### Current journey transition audit (final26/final35, 2026-09-22)

This current classification separates implementation and contract presence from
runtime acceptance. Final26's retained evidence
(`/var/tmp/sessionchat-browser-final26/cooking-execution.OpuIor.log:478-480`)
proves the authorized new-chat composition, release-owned adapter, installed
UI surface, and canonical host validation for two turns, then records restart
state. Final27 is retained as historical evidence: after correcting final26's
project-page lookup, it still failed before launching its installed UI, with
insufficient sanitized detail to establish why the card lookup failed
(`/var/tmp/sessionchat-browser-final27/cooking-execution.FGQAty.log:489-500`).
Final30 then proves that the recovery card and launch succeed, the installed
document returns HTTP 200, and recovery later fails while waiting for the
document URL (`/var/tmp/sessionchat-browser-final30/cooking-execution.FSdzbp.log`,
`browser.hfeK2a/playwright-results/session-chat-recovery-card-diagnostic.json`).
Final31 passes recovery and the canonical third turn, initializes the
concurrency clients, and fails in the concurrent race; fork and separate
negative phases are not reached. Final32 captures both concurrent requests,
then fails while checking a ref before either release completes. Final33
passes initial, recovery, and concurrency browser stages with canonical
validation for five turns, then fails in fork setup before fork browser
execution. Final35 passes all four browser phases with canonical validation for
the initial two turns, recovery third turn, concurrency fifth turn, and fork
target sixth turn; its separate negative process fails before its golden
scenario starts.

| Transition | Responsibility boundary | Current classification and evidence |
| --- | --- | --- |
| User intent → trusted creation | Platform composes the authorized new-chat route, repository/session resources, capability, installation, and handoff; the release owns its initialization contract. | **Supported for the exercised initial path.** Final26 created and opened the real Git-backed session. This does not establish a generic platform session API beyond that route. |
| Trusted creation → release-owned UI | Platform supplies the repository-scoped installation and verified route context; the release adapter owns its transcript and controls. | **Supported for the exercised initial path.** The installed UI initialized and served the session in final26. |
| Release UI input → human Git receive | The release adapter writes the human record; the platform authenticates and persists the receive with repository/ref attribution. | **Supported for two canonical turns.** Final26's host validation and retained receive evidence cover both turns; this is runtime evidence for the selected journey, not proof of every adapter operation. |
| Human receive → authorized run → isolated VM/broker | Platform selects the authorized trigger, immutable snapshot, and VM; the release agent reads its protocol and uses brokered model egress. | **Supported for the two-turn initial path.** Final26's canonical host validation follows both turns through the production bootstrap. |
| Assistant response Git → visible browser response/reconnect | The release adapter reads committed history and correlates responses; platform owns installation and route delivery. | **Supported for final26's initial browser path.** Two responses and the captured restart state are recorded; final31 additionally proves a third turn and reconnect after restart. |
| Restart state → recovery UI launch and reconnect | Platform/fixture must resolve the repository-scoped installation and handoff; the release UI must reconnect and reread history. | **Supported by final31.** Recovery and canonical validation of the third turn pass after correcting the harness URL assertion. Bootstrap preserves permitted theme query parameters; the earlier bare-path suffix assertion was invalid. |
| Recovery → concurrent writers | The platform reruns the release with the same authority boundary; the release handles expected-parent retry and preserves transcript history. | **Supported for the exercised browser phase.** Final33 passes all concurrency browser stages and canonical validation for five turns after the parser correction. Negative and full-journey acceptance remains open. |
| Recovery/history → forked session | Trusted composition creates fresh target authority, model binding, and installation; the release copies reachable history and applies its fork manifest. | **Supported for the exercised browser phase.** Final35 passes fork setup and the fork-target browser phase with canonical validation for the sixth turn after the role-idempotency and fresh-alias fixture corrections. |
| Journey → negative capability proof | Platform runs the separate fresh guest denial process; the release/runtime boundary supplies the scoped Git capability and fixed typed result. | **Attempted but unaccepted.** Final35 reaches the separate negative process, but it fails before the golden scenario at the restricted Caddy TLS published-proof setup; its typed summary records `status=failed`, `reason=marker_missing`, and `runner=101`. |

The platform-side receipt, UUID, and handoff prerequisites have separate
focused evidence: secret-binding receipt and replay are verified in the
PostgreSQL regression (lines 801-809 above); canonical lowercase UUIDs and
installation are verified in final17 (lines 826-833 above); and the route-base
handoff projection has a passing positive and mutation-negative test (lines
863-870 above). Final26 exercises those prerequisites in the initial path. The
final30 document-URL failure was therefore a browser assertion mismatch,
not a platform contract failure.

The smallest next step is to correct the negative child-environment isolation
and rerun the separate negative process. The
versioned protocol's
fork and tombstone warning semantics remain release-owned and do not add a
platform approval step (lines 1024-1027 above).

### Repository-wide quality gate (2026-09-22)

The corrected full `cargo dev quality` run passed with exit 0 in session
`52314`. It used disposable PostgreSQL and NATS services, enabled both real
isolated RPC proofs, and left all VM/Cooking/session-chat execution flags
unset. The earlier quality attempt that stopped at architecture diagnostics
(`DB-STATIC-SQL` and the scoped runtime/UI Git HTTP allowlist) is historical
and superseded by the reviewed narrow fixes; it is not a current gate
failure. The retained main log is
`/var/tmp/hephaestus-cargo-dev-quality-final2.log`, with service logs in
`/var/tmp/hephaestus-cargo-dev-quality-final2-postgres.log` and
`/var/tmp/hephaestus-cargo-dev-quality-final2-nats.log`.

The passing gate covers workspace formatting, Clippy, tests, documentation,
the real browser-session RPC lifecycle, the real UI-installation RPC matrix,
Cooking service checks, pinned Phoenix architecture/tests, and release UI
kit, installed-UI navigation, bridge, and UI architecture checks. Cleanup
removed the disposable containers and confirmed
`examples/session-chat/ui/node_modules` is absent. This is repository quality
evidence, not acceptance of the Cooking VM/GCP lifecycle or the full MVP-06
journey: final26 previously proved the initial browser path and two turns,
final35 now proves recovery, the canonical third turn, concurrency with five
canonical turns, and fork-target validation for the sixth turn. The separate
negative-process integration remains pending.

### Final28 browser evidence (2026-09-22)

The retained final28 run is
`/var/tmp/sessionchat-browser-final28/cooking-execution.NQ4Le3.log`
(`session47784`, exit 101). It passed the initial browser phase, canonical
host validation for two turns, and restart-state capture. Recovery then
failed during initialization after approximately 31.8 seconds; concurrency,
fork, and negative phases were not reached. The run cleaned up its processes
and containers.

This run does not prove that the repository-scoped installation card is
missing. The diagnostic hook covered only the card assertion, while the
preceding Phoenix-connected assertion was outside that hook, and the broad
failure catch can suppress a collection failure. Correct environment-path
forwarding was verified. The next smallest diagnostic is fixed, credential-
free breadcrumbs across the whole recovery initialization, including the
LiveView connection and category-specific fallback states. No recovery root
cause or full-journey acceptance is claimed from final28.

### Final30 recovery evidence (2026-09-22)

Final30 is retained at
`/var/tmp/sessionchat-browser-final30/cooking-execution.FSdzbp.log`
(`session86671`, exit 101), with the recovery diagnostic at
`browser.hfeK2a/playwright-results/session-chat-recovery-card-diagnostic.json`.
The initial browser phase, canonical two-turn validation, and restart-state
capture passed. Recovery now reaches the installation card and launches the
installed UI; its document returns HTTP 200, then initialization fails while
waiting for the document URL after about 31.8 seconds. Concurrency, fork, and
negative phases were not reached, and process/container cleanup was verified.

This supersedes final27's insufficient card-lookup diagnosis while retaining
final27 as historical evidence. The verified cause was the browser's old
bare-path suffix assertion: the bootstrap preserves the permitted
`heph_theme` and `heph_theme_origin` query parameters. Recovery, concurrency,
and fork assertions now check the pathname consistently with new-chat. Focused
TypeScript checks, four navigation tests, and four test-listing checks pass;
temporary diagnostics were removed. Final31 is recorded below; no concurrency
or full-journey acceptance is claimed until the race and later phases pass.

### Final31 recovery and concurrency evidence (2026-09-22)

Final31 is retained at
`/var/tmp/sessionchat-browser-final31/cooking-execution.lLeMay.log`
(`session81852`, exit 101). Initial two-turn validation, restart-state capture,
recovery browser execution, and canonical validation for the third turn pass.
The concurrent clients initialize successfully, but
`session_chat_concurrent_race` fails. Fork and negative phases are not reached;
process and container cleanup completed. Commit `7f95f72` contains the
launch-side correction. This is partial lifecycle evidence, not concurrency or
full-journey acceptance. The next step is a focused race diagnosis before a
fresh full lifecycle rerun.

### Final32 concurrency parser evidence (2026-09-22)

Final32 is retained as session `52053`, exit 101, with the diagnostic artifact
at
`/var/tmp/sessionchat-browser-final32/browser.AXTy8B/playwright-results/session-chat-concurrent-race-diagnostic.json`.
Initial two-turn validation, restart-state capture, recovery browser execution,
and canonical validation for the third turn pass. Both concurrent requests are
captured, but the race fails during ref checking before either release
completes. The harness parser regex has two capture groups while the checker
reads `match[3]`; the correct ref group is `match[2]`. The narrow harness fix
is recorded in commit `6a65296`, with runner/CI helper updates in `b8243c`.
This does not prove fork or full-journey acceptance; the next step is the fork
fixture correction followed by a fresh full lifecycle run.

### Final33 concurrency and fork-setup evidence (2026-09-22)

Final33 is retained at
`/var/tmp/sessionchat-browser-final33/cooking-execution.yXD1Pc.log`
(`session91574`, exit 101). Initial two-turn validation, recovery with the
canonical third turn, all concurrency browser stages, and canonical validation
for five turns pass. Fork setup then fails before fork browser execution on a
duplicate project-secret-manager role constraint. The negative phase is not
reached and cleanup completed. Commits `6a65296` and `b8243c` contain the
parser and runner/CI helper corrections; 26 Node checks and TypeScript checks
pass. This is not fork or full-journey acceptance. The next step is the narrow
fork-fixture role investigation followed by a fresh full lifecycle run.

### Final35 browser and negative-process evidence (2026-09-22)

Final35 is retained at
`/var/tmp/sessionchat-browser-final35/cooking-execution.ascqMI.log`
(`session27341`, overall exit 101). The initial, recovery, concurrency, and
fork browser phases pass with canonical validation for turns 2, 3, 5, and the
fork target 6. The main golden suite reports 35 passed, 0 failed, and 1
ignored. The fork fixture corrections in `ee67c2f` (role idempotency) and
`e4d8655` (fresh alias) are exercised by this run.

The separate negative process reaches its launch but fails before the golden
scenario at `golden.rs:4737`, where the Caddy TLS restricted published-proof
setup is unavailable. Its typed summary exists with
`status=failed`, `reason=marker_missing`, and `runner=101`; retained evidence is
`/var/tmp/sessionchat-browser-final35/session-chat-negative.C4IDsl.log` and
`session-chat-negative-summary.json`. Cleanup completed. This proves the full
browser chain, not negative capability acceptance or the GCP workflow; the
next step is child-environment isolation correction and a fresh rerun.
