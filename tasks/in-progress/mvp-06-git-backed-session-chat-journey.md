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
  - [ ] Record ownership of the visible transcript separately from
    agent-owned model context and internal workflow state.

- [ ] **2. Define the reference chat release's repository protocol**
  - [x] Document the reference release's session layout, message identity/order,
    user and agent records, correlation IDs, content references, branch/fork
    rules, concurrent-writer behavior, and compatibility/versioning behavior.
  - [x] Define release-owned initialization, participant policy,
    retention/tombstone behavior, and safe repository fork semantics.

- [ ] **3. Build the chat distribution flow**
  - [ ] Provide explicitly declared and installation-acknowledged repository
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
  - [ ] Persist runtime receive provenance from the immutable authority
    snapshot, suppress the originating attachment and reject downstream
    trigger candidates without explicit execution authority.
  - [ ] Build and publish a small ordinary chat-agent release that defines and
    reads its session protocol, calls its model API through MVP 04, and
    commits its response with normal Git.
  - [ ] Bind only the session repository/ref/path capability and its declared
    MVP 04 destination-bound egress bindings; prove that the release cannot
    use its source repository, another session, or an undeclared destination.

- [ ] **5. Prove the journey**
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
Review identified a remaining runtime-context dependency: the guest's
repository/ref/commit fields must describe the authorized target rather than
inherit the trigger repository's context. That correction is in progress.

The completed task records the released protocol version and source revision;
browser and real-Git evidence for session creation, turns, restart, and fork;
the reference agent's allowed session-repository push and denied cross-repository
or prohibited-write attempts; and fake-model egress evidence showing a
placeholder in the guest and no real provider token in guest-visible artifacts.
It also records the verification commands, results, and any explicitly
justified test-environment exclusions.
