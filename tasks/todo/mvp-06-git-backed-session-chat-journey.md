# MVP 06: Git-backed session chat journey

Owner: unassigned

## Outcome

Prove the main Hephaestus distribution UX: a user starts a chat session backed
by a repository, uses the selected release's chat adapter to submit a message,
and an ordinary released agent reads, commits, and pushes its response with a
tightly scoped Git capability. The agent calls its model API through MVP 04
placeholder substitution and never sees provider credentials.

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
| UI | Chat is the primary distribution UX for the selected release. The host supplies generic authenticated repository access and attribution; any message rendering, commands, forms, or richer UI remain release/distribution adapters. |
| Model | Automated coverage uses a deterministic fake model HTTPS API. A real OpenRouter smoke test is optional and uses MVP 04 destination-bound placeholder substitution. |

## Dependencies

- [`mvp-01.2-replace-controlled-result-publication-with-runtime-git.md`](mvp-01.2-replace-controlled-result-publication-with-runtime-git.md)
- [MVP 04: destination-bound HTTPS egress](mvp-04-brokered-model-and-outbound-capabilities.md)

## Implementation checklist

- [ ] **1. Define the reference chat release's repository protocol**
  - [ ] Document the reference release's session layout, message identity/order,
    user and agent records, correlation IDs, content references, branch/fork
    rules, concurrent-writer behavior, and compatibility/versioning behavior.
  - [ ] Define release-owned initialization, participant policy,
    retention/tombstone behavior, and safe repository fork semantics.

- [ ] **2. Build the chat distribution flow**
  - [ ] Add the authorized “new chat” workflow: create repository, create the
    session instance, bind its repository capability, let the release initialize
    its repository, and open the release's chat route.
  - [ ] Add the reference distribution's chat adapter that renders its own
    repository history, writes user input through its own repository contract,
    displays receive state, and shows committed agent responses.
  - [ ] Keep that adapter and any commands/forms out of the Hephaestus core
    workflow model.

- [ ] **3. Build the reference chat release**
  - [ ] Build and publish a small ordinary chat-agent release that defines and
    reads its session protocol, calls its model API through MVP 04, and
    commits its response with normal Git.
  - [ ] Bind only the session repository/ref/path capability and its declared
    MVP 04 destination-bound egress bindings; prove that the release cannot
    use its source repository, another session, or an undeclared destination.

- [ ] **4. Prove the journey**
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

## Non-goals

This task does not add a platform-owned session protocol, public HTTP ingress,
mailboxes, WebSockets, streaming model output, generic file upload, a universal
workflow engine, or unrestricted custom HTML. Those may become release adapters
over repositories and content capabilities later.

## Verify and document

- Publish the reference release's versioned repository protocol, including its
  retention and fork warning semantics, rather than presenting it as a core
  Hephaestus contract.
- Run real-Git and browser integration coverage for the journey and negative
  capability cases listed above. Exercise the released agent in its VM runtime
  with the deterministic fake HTTPS model endpoint; keep an OpenRouter smoke
  test separately opt-in and use only placeholder substitution.
- Run `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets --all-features`,
  `cargo test --workspace --all-features`, and
  `cargo doc --workspace --all-features --no-deps`. Run applicable UI checks
  when the reference distribution adapter changes.
- Before repository handoff, run `git diff --check` and `cargo dev quality`.

## Completion evidence

The completed task records the released protocol version and source revision;
browser and real-Git evidence for session creation, turns, restart, and fork;
the reference agent's allowed session-repository push and denied cross-repository
or prohibited-write attempts; and fake-model egress evidence showing a
placeholder in the guest and no real provider token in guest-visible artifacts.
It also records the verification commands, results, and any explicitly
justified test-environment exclusions.
