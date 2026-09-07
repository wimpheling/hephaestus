# MVP 05: Golden cooking-agent journey

Owner: codex

## Outcome

Prove the “own the loop” product end to end with a small family cooking agent
implemented as ordinary released software.

Two authorized family members communicate through released Telegram gateway
code. The cooking agent owns its model loop and persistent recipe memory,
updates a static cooking blog through controlled Git publication, uses its
declared model API and an application-owned Telegram relay without receiving
provider credentials, survives a
release update, and exposes exact release, state-volume, lease, authorization,
trigger, capability-use, and result provenance.

This is the acceptance task for MVP 01 through MVP 04, not a source of new
platform-specific Telegram or cooking abstractions.

## Locked decisions

| Area | Decision |
| --- | --- |
| Application ownership | The gateway, cooking-agent, and Telegram-relay repositories own Telegram semantics, user mapping, prompts, memory schema, model loop, retry/idempotency policy, blog generation, relay delivery, and user experience. |
| Users | Exactly two configured family identities are authorized in the golden path; an unrecognized identity is rejected by released policy before it reaches project authority. |
| Gateway boundary | The public gateway is a separate principal with a synchronous HTTP handler under the shared Caddy `/gateway/` namespace. It may publish only to the cooking-agent mailbox and has no repository or cooking-agent state authority. |
| Agent authority | The cooking agent may consume its mailbox, use its state, call its model API and Telegram relay through explicitly bound HTTPS destinations, and read/propose changes to one blog repository. |
| Publication | Canonical Git mutation remains a host-side controlled operation with exact target commit and authorization provenance. |
| Credentials | Model and Telegram-inbound verification credentials remain brokered and unavailable as raw guest values. The cooking agent receives only a brokered credential for the Telegram relay. The deterministic relay requires no Telegram Bot API token. The gateway receives only a non-secret inbound-verification placeholder. |
| State | Recipe memory is durable application state in the instance volume; the process may stop and reconstruct from state and mailbox events. |
| Testability | Deterministic model and relay-transport tests are required for automation. The E2E suite must run reproducibly locally and in CI without Telegram accounts, Bot API tokens or real provider delivery. |

## Acceptance-fixture specification

This specification defines application behavior for the golden journey. The
platform supplies only the generic released-workload, authority, mailbox,
state, brokered-egress, gateway, and controlled-Git capabilities described by
the preceding MVPs. The applications own every protocol and product decision
below.

### Application layout and identities

The fixture uses one `cooking` project and the following application-owned
source repositories. The Hugo toolchain is published separately so ordinary
blog content commits do not queue another OCI image build.

| Repository | Released application or content | Responsibility |
| --- | --- | --- |
| `cooking-gateway` | Telegram-style gateway | Validates inbound requests, applies application user policy and deduplication, and publishes a bounded mailbox event. |
| `cooking-agent` | Stateful cooking agent | Maintains recipe memory, invokes the model and outbound messaging APIs, renders the blog, and owns application retries/idempotency. |
| `telegram-relay` | External outbound Telegram relay | Authenticates cooking-agent requests and records deterministic delivery outcomes outside Hephaestus guests. |
| `cooking-blog` | Hugo static cooking blog | Receives controlled result proposals on `refs/heads/main`; its pinned Hugo build produces the HTML release artifact, and it contains no agent credentials or platform control data. |
| Hugo toolchain | Pinned OCI build image | Contains the reviewed Dockerfile, Hugo inputs and image declaration used by the blog build. |

Exactly two configured family identities, `alice` and `bob`, are authorized.
Their provider-user IDs are stable fixture values recorded during installation.
Every other identity is rejected by the released gateway before it can publish
a mailbox event or cause any cooking-agent, repository, state, or outbound
HTTPS operation.

### Gateway contract

The public route is `POST /gateway/cooking/telegram`. It accepts a bounded
Telegram-style update with a provider update ID, sender ID, and text body. The
gateway application:

1. requires the inbound verification header to equal the configured
   non-secret placeholder;
2. parses and bounds the update before application processing;
3. deduplicates by provider update ID in its own released application data;
4. maps only `alice` and `bob` to the application identities; and
5. publishes at most one `cooking.telegram.received.v1` event to the cooking
   mailbox.

The Caddy/brokered-secret boundary replaces a valid real verification header
with the placeholder before it reaches gateway code. The gateway never
receives, stores, logs, or compares a raw verification secret.

The synchronous response contract is deliberately bounded:

| Input outcome | Response | Side effect |
| --- | --- | --- |
| Valid new update from `alice` or `bob` | `200` acknowledgement | One normalized cooking-mailbox event. |
| Valid duplicate update | `200` acknowledgement | No second mailbox event. |
| Missing or invalid verification | `401` | None. |
| Verified update from any other user | `403` | None. |
| Malformed or over-limit update | `400` | None. |

The event contains only the stable provider update ID, application user ID,
normalized command, and bounded text. Raw provider bodies, verification
headers, and provider metadata are not projected into product-event streams.
The gateway may publish only to this one mailbox; it has no authority over the
cooking-agent state volume, blog repository, model API, Telegram relay, or
Caddy administration.

### Cooking-agent contract

The cooking agent consumes its durable mailbox serially under its instance
state lease. For each new normalized request it performs one SQLite
transaction that records the provider update ID, updates recipe/conversation
memory, and creates the pending recipe work. Replayed mailbox events find the
existing ledger record and have no second logical effect.

The state database contains these application-owned tables:

| Table | Purpose |
| --- | --- |
| `schema_meta` | One current application schema version. |
| `processed_updates` | Provider update ID, disposition, and result identity for transactional deduplication. |
| `users` | The two application identities and bounded preferences. |
| `recipes` | Stable recipe identity, request summary, rendered content, and publication outcome. |
| `conversation_summaries` | Bounded per-user context used for later model requests. |

For an accepted request the agent builds a bounded context from the requesting
user's summary and recipe history, calls the declared deterministic model API,
validates its bounded response, updates SQLite, renders a deterministic recipe
page and index entry, and sends a bounded outbound response through the
declared Telegram relay. Every accepted recipe request immediately creates or
updates its deterministic blog page; there is no separate `/publish` command
in the golden path.

The blog is a conventional Hugo source repository: `hugo.toml`,
`content/_index.md`, `content/recipes/_index.md`,
`content/recipes/<stable-slug>.md`, committed layouts and static assets, and
no generated output in Git. The cooking agent renders deterministic Markdown
recipe pages only. It runs the repository's declared static-site check before
proposing a result; the blog's pinned Hugo build image runs `hugo` to produce
the immutable `public/` HTML artifact for static serving. It uses the exact
incoming `refs/heads/main` commit and the platform's controlled Git-result
operation; released code cannot mutate canonical Git directly.

The agent has only its own mailbox and state volume, the exact blog repository
and ref, and the two declared brokered HTTPS destinations: model API and
Telegram relay. It cannot inspect
raw credentials, use ordinary TCP/DNS, access another project or mailbox,
broaden a destination, bind another repository, alter authorization, or
administer Caddy.

### Secrets, egress, and resource policy

The fixture declares three brokered secret uses:

| Use | Principal | Boundary contract |
| --- | --- | --- |
| Telegram inbound verification | Gateway | The route rewrites a valid inbound header to the released-code placeholder. |
| Model API | Cooking agent | An exact HTTPS destination and request-header placeholder are substituted only at the host broker. |
| Telegram relay API | Cooking agent | An exact relay origin and request-header placeholder are substituted only at the host broker. |

The relay accepts only a bounded application message request and a stable
idempotency key. It authenticates the cooking agent and uses deterministic
transport to record delivery outcomes, returning a redacted bounded response.
Failure tests inject rejection, lost responses and uncertain outcomes to exercise
Hephaestus broker behavior and application recovery without a live provider.

Model and relay responses are deterministic. Ingress uses simulated Telegram-style
updates from fixture identities; no real Telegram accounts, Bot API tokens,
messages or Internet-reachable deployment are required by MVP-05.
Both released
guests are network-disabled except for their declared boundary: the gateway
has its private HTTP handler and mailbox publication; the cooking agent uses
only brokered HTTPS. Resource limits, ordinary parameters, capability
requirements, and secret-slot bindings are recorded in each immutable release
and instance revision.

### State update and recovery contract

The fixture publishes a second compatible cooking-agent release. Its update
hook upgrades SQLite from schema version 1 to version 2 by adding a
`recipe_summary` field and index, backfilling existing rows transactionally,
and updating rendered blog output to show the summary. The hook is idempotent:
re-entry after a crash observes the completed version or safely completes the
same migration. A deliberate nonzero hook fixture reports only after its own
application rollback; abnormal termination leaves the platform's documented
paused/recovery state rather than claiming an application rollback.

Stopping the cooking-agent process after a successful request, then delivering
another request, must reconstruct behavior from SQLite and durable mailbox
delivery alone. It must not depend on a process checkpoint, unrecorded memory,
or a hidden provider-side deduplication guarantee.

### Required acceptance assertions

The recorded fixture evidence must prove all of the following:

- concurrent requests from `alice` and `bob` receive the gateway contract and
  produce serialized, idempotent cooking-agent effects;
- all valid model and relay calls use only their exact declared HTTPS bindings,
  no guest observes a raw fixture credential, and deterministic relay outcomes
  remain inspectable;
- each accepted recipe creates a controlled proposal/result from the exact
  blog target commit, and canonical Git changes only through the host-side
  publisher;
- invalid verification, malformed input, unknown identities, duplicate
  delivery, revoked authorization, unbound destination use, direct Git writes,
  and cross-project/resource access have the stated denial or replay outcome;
- rotation selects new secret versions only for later operations while older
  run provenance remains resolvable;
- the compatible update drains/defer events, runs under the exclusive state
  lease, activates the new revision, then resumes deferred work; and
- one end-to-end inspection resolves route, gateway revision, mailbox event,
  cooking revision, authorization snapshot, state lease, HTTPS uses, Git
  result, final disposition, and redacted historical data for an authorized
  viewer only.

## Dependencies

- [Runtime authority](../../tasks/done/mvp-01-agent-principals-capabilities-and-runtime-authority.md)
- [Runtime Git](../../tasks/done/mvp-01.2-replace-controlled-result-publication-with-runtime-git.md)
- [Durable mailboxes](../../tasks/done/mvp-02-durable-agent-mailboxes-and-stateful-dispatch.md)
- [Gateway ingress](../../tasks/done/mvp-03-event-ingress-and-caddy-routing.md)
- [Brokered capabilities](../../tasks/done/mvp-04-brokered-model-and-outbound-capabilities.md)
- [Gateway-to-mailbox publication](../../tasks/in-progress/gateway-to-mailbox-publication.md)
- [Product definition](../../tasks/todo/define-own-the-loop-agent-platform.md)

## Non-goals

This task does not produce a universal assistant, beginner distribution,
catalog, Operator/Admin Agent, Project Agent, web search, browser use,
WebSockets, streaming services, general scheduling, real-provider availability
guarantees, real Telegram transport or account integration, Bot API token
management, public Internet deployment, or production marketing material.

## Implementation checklist

Checked items below are backed by the deterministic installed-artifact journey
and focused tests recorded on 2026-09-05. Parent items stay open when any part
is unfinished. Local fixture installation is not evidence of the full release
build/install workflow, and application unit tests are not evidence of the
complete real-stack restart/update/crash matrix.

The actionable remaining-work plan is
[Complete MVP-05 acceptance](../../tasks/in-progress/mvp-05.1-complete-cooking-acceptance.md).
This document remains the acceptance specification; the task records sequencing,
implementation gaps, and the evidence required to close its remaining items.

Scope decision, 2026-09-07: the user moved the exhaustive
[host-daemon crash matrix](../../tasks/todo/complete-host-daemon-crash-recovery-matrix.md)
and [expanded adversarial isolation matrix](../../tasks/todo/complete-adversarial-isolation-e2e-matrix.md)
into separate tasks, outside MVP-05 completion criteria. Existing executable
guest-crash, denial, isolation, rotation/revocation, retirement and confinement
checks remain required. Known security defects remain blockers. This is an
explicit deferral, not a claim of completed coverage.

- [x] **1. Specify the complete acceptance fixture**
  - [x] **Define released application behavior**
    - [x] Specify the gateway request validation, Telegram update parsing,
      stable application deduplication, two-user mapping, normalization,
      HTTP acknowledgement/status contract, and cooking-mailbox publication
      contract. Specify the exact Telegram brokered placeholder slot, inbound
      header rule, rotation behavior, and rejected-request responses.
    - [x] Specify the cooking agent's model loop, bounded context, recipe
      SQLite schema, transaction and idempotency policy, blog rendering, and
      outbound response behavior.
    - [x] Specify the Hugo cooking-blog source layout, pinned static-build
      image, build/check commands, immutable HTML artifact, result proposal,
      approval, and controlled publication flow.
    - [x] Specify stable release configuration for parameters, capability
      requirements, secret slots, state, resource bounds, network ceiling,
      runtime commands, and update hook.
  - [x] **Define allowed and denied authority**
    - [x] Record the exact gateway and cooking-agent capability declarations,
      concrete bindings, grants, HTTPS destinations/substitution rules, repository,
      mailbox, route, secrets, and state volume.
    - [x] Record explicit denials for other users, projects, repositories,
      mailboxes, routes, secrets, undeclared API destinations, authorization changes,
      direct canonical Git writes, and Caddy administration.
    - [x] Define acceptance assertions for every allowed and denied operation.

- [ ] **2. Build and publish the reference releases**
  - [x] Create small reviewable gateway, cooking-agent, and outbound relay
    application sources without privileged framework dependencies. They now
    live together under `examples/cooking/`; creating their forge repositories
    through the real build/install workflow remains below.
  - [ ] Build the gateway and cooking agent in isolated build guests and
    publish immutable releases with
    exact source, build, artifact-manifest, runtime-policy, capability, and
    secret-slot provenance.
  - [ ] Run the relay outside the guests as part of the disposable test stack,
    with deterministic transport, a bounded authenticated request contract and
    redacted logs.
  - [ ] Create a second compatible cooking-agent release with a real state
    update hook and a visible behavior or schema change.
  - [x] Add unit and conformance tests for protocol parsing, user policy,
    application deduplication, recipe transactions, blog rendering, update
    idempotency, and rollback.
  - [x] Prove normal guests execute imported read-only artifacts rather than
    source trees or runtime-downloaded executable dependencies.

- [ ] **3. Install and bind the product slice**
  - [ ] Import the gateway and cooking releases into one project as distinct
    instances and immutable revisions.
  - [x] Allocate cooking-agent state and its durable mailbox without giving
    either resource to the gateway.
  - [x] Bind a public `/gateway/` Caddy route to the gateway's synchronous HTTP
    handler, record its resolved URL, and bind gateway publication only to the
    cooking mailbox.
    Verified on the local Caddy listener; Internet-reachable deployment is
    outside MVP-05 scope.
  - [x] Bind three separate fixture secrets for model, relay and inbound
    verification, with exact host-side substitution and no raw guest values.
  - [ ] Create and bind model-API, Telegram-relay, and Telegram-verification
    secrets without exposing values to the binding user or either guest. Bind
    their exact placeholder, destination, and gateway-route substitution rules.
    The deterministic relay uses its fixture credential and requires no Bot API
    token or Telegram account.
  - [x] Bind the cooking agent to one exact blog repository/ref and bounded
    HTTPS destination and placeholder-substitution bindings.
  - [ ] Record the exact installation, revision, attachment, route,
    authorization snapshot, state volume, fenced lease, dispatch order, and
    secret binding fixture IDs.
    The disposable run's redacted route/revision/mailbox/run/lease/version/result
    evidence is recorded; retain the complete installation manifest when the
    real build/import workflow is exercised.

- [ ] **4. Exercise normal operation**
  - [x] Send one simulated Alice request through real Caddy/libkrun, exercise
    brokered model and actual deterministic relay code, persist SQLite state,
    and produce a recipe. Check missing/invalid verification and unknown-user
    responses through this same gateway.
  - [ ] Send simultaneous simulated Telegram-style requests from both fixture users
    through Caddy and receive the handler's specified bounded HTTP responses.
  - [ ] Send valid, missing, invalid, and rotated-secret Telegram requests and
    verify that the authorized inbound header is rewritten to the placeholder,
    gateway repository code returns the specified responses, and no cooking
    agent, repository, HTTPS egress, or state authority is used before rejection.
  - [x] Verify the gateway normalizes and publishes only the expected bounded
    events to the cooking mailbox.
  - [ ] Verify stateful cooking runs serialize, call the declared model API and
    Telegram relay through destination-bound placeholder substitution, update
    recipe memory transactionally, and handle ordinary API responses in
    repository code.
  - [x] Verify a generated blog change uses the exact target commit, creates a
    controlled proposal/result, and reaches canonical Git only through the
    authorized host-side publisher.
  - [ ] Stop the cooking process, deliver another message, and verify restart
    from durable state and mailbox replay without process checkpointing.
  - [ ] Deliver duplicate ingress and NATS events and verify one logical
    application effect despite at-least-once platform delivery.

- [ ] **5. Prove the authority boundary**
  - [x] Send an event from an unauthorized Telegram identity and verify
    rejection without cooking-agent, repository, HTTPS egress, or state authority.
  - [ ] Preserve and execute existing gateway and cooking-agent adversarial
    denial checks with exact identities and no unauthorized effects. Complete
    foreign-resource, direct-network/Git, authority-change and Caddy coverage
    in the linked adversarial task; its expanded matrix and missing positive
    controls are explicitly deferred and are not MVP-05 completion blockers.
  - [ ] Rotate the Telegram verification, relay-authentication, and model
    credentials and prove later operations use the new exact versions while
    earlier run provenance remains intact.
  - [ ] Revoke broker authority during an active journey and verify live denial,
    honest in-flight semantics, durable audit, and safe recovery.

- [ ] **6. Update and recover the stateful agent**
  - [x] Verify the application's v1-to-v2 SQLite migration, re-entry and
    explicit rollback in unit tests; separately verify gateway acceptance
    during closed update gates and dispatch after reopening in PostgreSQL.
  - [ ] Start the second cooking-agent release update, close the run gate,
    accept and defer simultaneous Telegram events, drain old runs, and acquire
    the exclusive state lease.
  - [ ] Execute the update hook in an isolated guest, activate the candidate,
    reopen the gate, and bind deferred events only to the new revision.
  - [ ] Verify recipes, authorized users, route, mailbox, attachment, and
    instance identity survive the update.
  - [ ] Exercise explicit hook rollback and abnormal termination fixtures and
    verify the documented runnable or paused states without false host rollback
    claims.
  - [ ] Recover the paused fixture through the authorized operator path and
    retain every historical revision, update, event, state-access outcome, and
    audit record.

- [ ] **7. Inspect exact provenance**
  - [x] From the project UI or inspection API, resolve one journey from public
    request through route, gateway revision, normalized mailbox event,
    cooking-agent revision, authorization snapshot, state volume, fenced
    lease, dispatch order, state-access outcome, HTTPS egress uses, Git
    result, and final disposition.
  - [x] Deny outsider and wrong-audience inspection of the cooking run;
    verify secret metadata permission filtering, pagination and retained
    secret-version history with focused PostgreSQL tests.
  - [ ] Verify tombstoning an attachment or revoking a release, route, grant,
    or secret preserves historical resolution while denying new unauthorized
    work.
  - [ ] Verify unauthorized viewers cannot inspect request/message bodies,
    state contents, parameters marked sensitive, secret metadata, provider
    payloads, or hidden project resources through provenance views or live
    updates.
  - [x] Capture redacted fixture IDs and application hashes in the example
    README, clearly identifying them as evidence from disposable resources.
  - [ ] Capture browser screenshots suitable for technical product
    documentation without including secret or private family data.

- [ ] **8. Automate the real-system journey**
  - [x] Add a runnable deterministic single-request Caddy/libkrun/PostgreSQL/
    NATS test with brokered calls, controlled approval, authenticated
    inspection and cleanup; run it with the optional pinned Hugo HTML check.
  - [ ] Add a real-PostgreSQL and NATS integration scenario covering install,
    binding, concurrent ingress, stateful dispatch, Git publication,
    update/recovery, revocation, and exact provenance.
  - [ ] Add a real-Caddy and real-libkrun scenario for gateway and cooking-agent
    isolation, mounts, networking, broker use, restart, and cleanup.
  - [ ] Add a Playwright journey covering project navigation, installation,
    binding, operation, update, denial, recovery, and provenance inspection.
  - [ ] Execute existing guest-crash cases around state persistence, broker
    calls and proposal-ready state, plus rollback and abnormal-update recovery.
    Exhaustive host-daemon interruption around ingress commit, dispatch, result
    publication, update completion, activation and cleanup belongs to the linked
    crash task and is not an MVP-05 completion requirement.
  - [ ] Scan PostgreSQL, NATS, logs, traces, metrics, filesystems, browser
    payloads, screenshots, VM environment, files, and process arguments for
    model-API, relay-authentication and inbound verification-secret sentinels.

- [ ] **9. Verify and document**
  - [x] Document how the reference applications own their loops and protocol
    semantics while platform authority remains external.
  - [ ] Document how to reproduce the deterministic local journey and inspect
    every allowed, denied, update, and recovery result.
  - [x] Document how to locate and reproduce the current deterministic example
    and application-only tests from `examples/cooking/README.md`.
  - [x] Run `cargo fmt --all -- --check`.
  - [x] Run `cargo clippy --workspace --all-targets --all-features`.
  - [x] Run `cargo test --workspace --all-features`.
  - [x] Run `cargo doc --workspace --all-features --no-deps`.
  - [x] Run `cargo dev quality` on the consolidated example: architecture,
    protobuf, Rust, Phoenix (233 tests) and focused UI (92 tests) passed.
    These are baseline results, not completion of the opt-in acceptance matrix;
    rerun required checks after the remaining implementation changes.
  - [ ] Run real-PostgreSQL, NATS, Caddy, libkrun, broker, update, Git, and
    failure-injection scenarios.
  - [ ] Run `mix precommit` in `web/`.
  - [ ] Run the Playwright browser project.
  - [ ] Run Mélange drift detection, `melange doctor`, and OpenFGA
    compatibility fixtures.
  - [ ] Run secret-sentinel scans and `git diff --check`.

## Completion evidence

### Deterministic installed-artifact slice, 2026-09-05

Implemented and verified the focused first-request slice; the complete
acceptance checklist above remains open. Reproduction, exact artifact hashes
and redacted fixture IDs are recorded in
[the example README](README.md).

The real Caddy/libkrun/PostgreSQL/NATS scenario now accepts one authorized
normalized cooking event, runs the exact imported cooking application against
durable SQLite, calls a deterministic TLS model and the actual external relay
application through two broker rules, creates a Markdown result, approves it
through the authorized host RPC, and builds the approved recipe with pinned
Hugo. Authenticated inspection resolves route, gateway revision, event, run,
authorization snapshot, state lease, both exact HTTPS uses and final result;
outsider and wrong-audience inspection are denied.

The implementation closed the audit's update-ingress and `401` mismatches and
the concrete gaps exposed by the real request: sealed immutable gateway
parameters; frozen mailbox target context and workspaces; actual correlated
HTTPS audit recording; mailbox-run inspection; and controlled review proposals
for mailbox results, including historical recovery through migration 0062.

Verification passed the cooking wrapper (one daemon journey plus four real
PostgreSQL/NATS tests), the ordinary gateway wrapper (same counts), focused
gateway/runtime tests, and the combined real-PostgreSQL frozen-target/workspace/
proposal regression. Application conformance and pinned Hugo checks are
recorded in the application repositories. The workspace quality gate is
recorded separately at handoff.

This fixture imports exact application artifacts and seeds release metadata;
isolated build/publication of all reference releases remains separate work.
The retained concurrency, revocation, guest-crash, update/recovery and browser
matrix and reproducible local and CI execution are still required before
completing MVP-05. The exhaustive host-daemon crash and expanded adversarial
matrices are owned by the separate tasks linked above. Real Telegram integration
is excluded from acceptance.

Record source repository commits, build/release/instance/revision IDs, route
and mailbox IDs, state-volume and fenced-lease IDs, dispatch order and
state-access outcomes, authorization snapshots, secret versions and leases,
HTTPS egress usage records, Git target/result commits, update and recovery
IDs, denial evidence, test counts, screenshots, and exact verification
commands.
