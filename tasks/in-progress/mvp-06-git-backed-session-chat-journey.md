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

The completed task records the released protocol version and source revision;
browser and real-Git evidence for session creation, turns, restart, and fork;
the reference agent's allowed session-repository push and denied cross-repository
or prohibited-write attempts; and fake-model egress evidence showing a
placeholder in the guest and no real provider token in guest-visible artifacts.
It also records the verification commands, results, and any explicitly
justified test-environment exclusions.
