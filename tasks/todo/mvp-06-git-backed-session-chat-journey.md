# MVP 06: Git-backed session chat journey

Owner: unassigned

## Outcome

Prove the main Hephaestus distribution UX: a user starts a chat session backed
by a repository, sends a message through the chat UI, and an ordinary released
agent reads, commits, and pushes its response with a tightly scoped Git
capability. The agent invokes a model through the MVP 04 broker and never sees
provider credentials.

The chat UI is a distribution-layer adapter over the session repository. It is
not a platform-owned universal prompt, workflow, or form model.

## Locked decisions

| Area | Decision |
| --- | --- |
| Session state | Every session has one project repository whose Git history is the durable, forkable conversation record. |
| Installation | Starting a session creates a fresh instance from a published chat release and binds its symbolic `session` repository requirement to that repository. No rebuild occurs. |
| User input | The authenticated chat UI records each user message as a normal attributed Git commit. The first message is therefore an ordinary repository event, not an instance parameter. |
| Agent output | The runtime uses normal Git to commit and fast-forward only the bound session branch. Its own receive does not trigger another turn. |
| Protocol | `heph.session/v1` is a small versioned repository layout containing session metadata and append-only user/agent message records. Attachments are immutable content references, not arbitrary bytes in message JSON. |
| UI | Chat is the primary distribution UX. Standard text, commands, and schema-declared forms are rendered by the host; richer release-provided UI is deferred to a sandboxed iframe host. |
| Model | Automated coverage uses a deterministic fake model. A real OpenRouter smoke test is optional and uses only the MVP 04 broker. |

## Dependencies

- [`mvp-01.2-replace-controlled-result-publication-with-runtime-git.md`](mvp-01.2-replace-controlled-result-publication-with-runtime-git.md)
- [`mvp-04-brokered-model-and-outbound-capabilities.md`](mvp-04-brokered-model-and-outbound-capabilities.md)

## Implementation checklist

- [ ] **1. Define the session repository protocol**
  - [ ] Specify `heph.session/v1`, message identity/order, user and agent
    records, correlation IDs, content references, branch/fork rules, and
    compatibility/versioning behavior.
  - [ ] Define session repository creation, participant authorization,
    retention/tombstone behavior, and safe repository fork semantics.

- [ ] **2. Build the chat distribution flow**
  - [ ] Add the authorized “new chat” workflow: create repository, import or
    create the session instance, bind its repository capability, initialize
    `heph.session/v1`, and open the chat route.
  - [ ] Add a chat page that renders repository message history, submits an
    attributed user commit, displays run/receive state, and shows committed
    agent responses.
  - [ ] Render bounded commands and schema-declared forms as structured next
    user messages without making forms a core workflow abstraction.

- [ ] **3. Build the reference chat release**
  - [ ] Build and publish a small ordinary chat-agent release that reads the
    session protocol, invokes the bounded model capability, and commits its
    response with normal Git.
  - [ ] Bind only the session repository/ref/path capability and the bounded
    model capability; prove that the release cannot use its source repository
    or another session.

- [ ] **4. Prove the journey**
  - [ ] Cover session creation, first message, agent response, subsequent
    turn, branch/fork, restart/recovery, and visibility/history in browser and
    real-Git integration tests.
  - [ ] Cover denied source/other-repository access, prohibited ref/path
    writes, delete/force-push attempts, expired/revoked Git capability, and
    recursive-trigger suppression.
  - [ ] Run deterministic fake-model coverage and a separate optional real
    OpenRouter broker smoke test without weakening credential controls.

## Non-goals

This task does not add public HTTP ingress, mailboxes, WebSockets, streaming
model output, generic file upload, a universal workflow engine, or unrestricted
custom HTML. Those may become adapters over sessions and repository/content
capabilities later.
