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

- [ ] **3. Build the chat distribution flow**
  - [x] Provide explicitly declared and installation-acknowledged repository
    Git access on the host-owned installed UI origin. Revalidate the live child
    session, exact repository target and human grants for each operation; never
    expose a platform credential to release content.
  - [ ] Add the authorized “new chat” workflow: create repository, create the
    session instance, bind its repository capability, let the release initialize
    its repository, and open the release's chat route.
  - [ ] Add the reference distribution's chat adapter that renders its own
    repository history, writes user input through its own repository contract,
    displays receive state, and shows committed agent responses.
  - [ ] Mount that adapter only through the completed release-owned
    distribution UI surface and its declared tab/API bindings.
  - [ ] Keep that adapter and any commands/forms out of the Hephaestus core
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
  - [ ] Bind only the session repository/ref/path capability and its declared
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
  - [ ] Cover denied source/other-repository access, prohibited ref/path
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

- [ ] Publish the reference release's versioned repository protocol, including
  its retention and fork warning semantics, rather than presenting it as a
  core Hephaestus contract.
- [ ] Run real-Git and browser integration coverage for the journey and
  negative capability cases listed above. Exercise the released agent in its
  VM runtime with the deterministic fake HTTPS model endpoint; keep an
  OpenRouter smoke test separately opt-in and use only placeholder
  substitution.
- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo clippy --workspace --all-targets --all-features`.
- [ ] Run `cargo test --workspace --all-features`.
- [ ] Run `cargo doc --workspace --all-features --no-deps`.
- [ ] Run applicable UI checks when the reference distribution adapter changes.
- [ ] Before repository handoff, run `git diff --check` and `cargo dev quality`.
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

Commit `b079c48` adds `examples/session-chat/tests/denied_probe.py` and five
focused Python tests. The probe uses the production runtime credential/helper
and broker contracts, first requiring an authorized deterministic model call
with the same credential and binding, then checking source-checkout absence and
other-repository Git denials, a positive authorized clone followed by a prohibited-path push,
and undeclared model destination/rule requests. Its adapter checks distinguish
wire `denied` from `retryable`, transport failure, and malformed responses;
output is fixed check/status metadata only. This probe is not yet wired into a
release image or executed in a real VM, so broad released-VM denial acceptance
remains unchecked.

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
