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
| Credentials | Model and Telegram-inbound verification credentials remain brokered and unavailable as raw guest values. The cooking agent receives only a brokered credential for the Telegram relay. The relay alone holds the raw Telegram Bot API token in its external secret store because Telegram requires it in a URL path. The gateway receives only a non-secret inbound-verification placeholder. |
| State | Recipe memory is durable application state in the instance volume; the process may stop and reconstruct from state and mailbox events. |
| Testability | Deterministic model and relay-transport tests are required for automation. A real Telegram smoke is required before MVP completion and uses the relay without weakening guest credential controls. |

## Acceptance-fixture specification

This specification defines application behavior for the golden journey. The
platform supplies only the generic released-workload, authority, mailbox,
state, brokered-egress, gateway, and controlled-Git capabilities described by
the preceding MVPs. The applications own every protocol and product decision
below.

### Application layout and identities

The fixture uses one `cooking` project and four application-owned source
repositories:

| Repository | Released application or content | Responsibility |
| --- | --- | --- |
| `cooking-gateway` | Telegram-style gateway | Validates inbound requests, applies application user policy and deduplication, and publishes a bounded mailbox event. |
| `cooking-agent` | Stateful cooking agent | Maintains recipe memory, invokes the model and outbound messaging APIs, renders the blog, and owns application retries/idempotency. |
| `telegram-relay` | External outbound Telegram relay | Authenticates cooking-agent requests, owns the raw Bot API token outside Hephaestus guests, and calls Telegram's real Bot API. |
| `cooking-blog` | Hugo static cooking blog | Receives controlled result proposals on `refs/heads/main`; its pinned Hugo build produces the HTML release artifact, and it contains no agent credentials or platform control data. |

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
idempotency key. It authenticates the cooking agent, then calls Telegram's
real Bot API using its raw bot token in the required URL path. The raw token
is stored and used only by the relay's external deployment; it is never a
Hephaestus secret binding, guest mount, application parameter, source file,
log value, or product event. The relay returns a redacted bounded outcome.

Deterministic model and relay-transport tests use fixed non-secret responses.
The required real Telegram smoke exercises the same relay request contract
with the two configured family accounts. It requires a public HTTPS ingress
for the gateway (for example, a deliberately configured tunnel during local
development) and must record no credential in its evidence. Both released
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
  no guest observes a provider credential, and the relay alone makes the
  real Telegram Bot API call;
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
guarantees, or production marketing material.

## Implementation checklist

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
  - [ ] Create small reviewable source repositories for the gateway, cooking
    agent, and outbound Telegram relay without privileged framework dependencies.
  - [ ] Build the gateway and cooking agent in isolated build guests and
    publish immutable releases with
    exact source, build, artifact-manifest, runtime-policy, capability, and
    secret-slot provenance.
  - [ ] Deploy the relay independently with its external raw-token secret
    store, bounded request contract, redacted logs, and Telegram API egress
    restricted to the intended provider origin.
  - [ ] Create a second compatible cooking-agent release with a real state
    update hook and a visible behavior or schema change.
  - [ ] Add unit and conformance tests for protocol parsing, user policy,
    application deduplication, recipe transactions, blog rendering, update
    idempotency, and rollback.
  - [ ] Prove normal guests execute imported read-only artifacts rather than
    source trees or runtime-downloaded executable dependencies.

- [ ] **3. Install and bind the product slice**
  - [ ] Import the gateway and cooking releases into one project as distinct
    instances and immutable revisions.
  - [ ] Allocate cooking-agent state and its durable mailbox without giving
    either resource to the gateway.
  - [ ] Bind a public `/gateway/` Caddy route to the gateway's synchronous HTTP
    handler, record its resolved URL, and bind gateway publication only to the
    cooking mailbox.
  - [ ] Create and bind model-API, Telegram-relay, and Telegram-verification
    secrets without exposing values to the binding user or either guest. Bind
    their exact placeholder, destination, and gateway-route substitution rules;
    provision the raw Bot API token only in the relay's external secret store.
  - [ ] Bind the cooking agent to one exact blog repository/ref and bounded
    HTTPS destination and placeholder-substitution bindings.
  - [ ] Record the exact installation, revision, attachment, route,
    authorization snapshot, state volume, fenced lease, dispatch order, and
    secret binding fixture IDs.

- [ ] **4. Exercise normal operation**
  - [ ] Send simultaneous real Telegram requests from both authorized users
    through Caddy and receive the handler's specified bounded HTTP responses.
  - [ ] Send valid, missing, invalid, and rotated-secret Telegram requests and
    verify that the authorized inbound header is rewritten to the placeholder,
    gateway repository code returns the specified responses, and no cooking
    agent, repository, HTTPS egress, or state authority is used before rejection.
  - [ ] Verify the gateway normalizes and publishes only the expected bounded
    events to the cooking mailbox.
  - [ ] Verify stateful cooking runs serialize, call the declared model API and
    Telegram relay through destination-bound placeholder substitution, update
    recipe memory transactionally, and handle ordinary API responses in
    repository code.
  - [ ] Verify a generated blog change uses the exact target commit, creates a
    controlled proposal/result, and reaches canonical Git only through the
    authorized host-side publisher.
  - [ ] Stop the cooking process, deliver another message, and verify restart
    from durable state and mailbox replay without process checkpointing.
  - [ ] Deliver duplicate ingress and NATS events and verify one logical
    application effect despite at-least-once platform delivery.

- [ ] **5. Prove the authority boundary**
  - [ ] Send an event from an unauthorized Telegram identity and verify
    rejection without cooking-agent, repository, HTTPS egress, or state authority.
  - [ ] Run an adversarial gateway release and prove it cannot inspect cooking
    state, read the blog repository, publish to another mailbox, broaden its
    route, inspect another project, or administer Caddy.
  - [ ] Run adversarial cooking-agent operations and prove they cannot read
    real credentials, bypass forced proxy egress, use an unbound destination,
    bind another repository, alter authorization, or write canonical Git
    directly.
  - [ ] Rotate the Telegram verification, relay-authentication, and model
    credentials and prove later operations use the new exact versions while
    earlier run provenance remains intact; rotate the relay's Bot API token
    independently without exposing it to a guest.
  - [ ] Revoke broker authority during an active journey and verify live denial,
    honest in-flight semantics, durable audit, and safe recovery.

- [ ] **6. Update and recover the stateful agent**
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
  - [ ] From the project UI or inspection API, resolve one journey from public
    request through route, gateway revision, normalized mailbox event,
    cooking-agent revision, authorization snapshot, state volume, fenced
    lease, dispatch order, state-access outcome, HTTPS egress uses, Git
    result, and final disposition.
  - [ ] Verify tombstoning an attachment or revoking a release, route, grant,
    or secret preserves historical resolution while denying new unauthorized
    work.
  - [ ] Verify unauthorized viewers cannot inspect request/message bodies,
    state contents, parameters marked sensitive, secret metadata, provider
    payloads, or hidden project resources through provenance views or live
    updates.
  - [ ] Capture stable fixture IDs and screenshots suitable for technical
    product documentation without including secret or private family data.

- [ ] **8. Automate the real-system journey**
  - [ ] Add a real-PostgreSQL and NATS integration scenario covering install,
    binding, concurrent ingress, stateful dispatch, Git publication,
    update/recovery, revocation, and exact provenance.
  - [ ] Add a real-Caddy and real-libkrun scenario for gateway and cooking-agent
    isolation, mounts, networking, broker use, restart, and cleanup.
  - [ ] Add a Playwright journey covering project navigation, installation,
    binding, operation, update, denial, recovery, and provenance inspection.
  - [ ] Inject crashes around ingress commit, dispatch, state commit, broker
    call, result publication, update hook, revision activation, and cleanup.
  - [ ] Scan PostgreSQL, NATS, logs, traces, metrics, filesystems, browser
    payloads, screenshots, VM environment, files, and process arguments for
    application-API and Telegram verification-secret sentinels; separately
    verify the relay's raw Bot API token is absent from its logs and evidence.

- [ ] **9. Verify and document**
  - [ ] Document how the reference applications own their loops and protocol
    semantics while platform authority remains external.
  - [ ] Document how to reproduce the deterministic local journey and inspect
    every allowed, denied, update, and recovery result.
  - [ ] Run `cargo fmt --all -- --check`.
  - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
  - [ ] Run `cargo test --workspace --all-features`.
  - [ ] Run `cargo doc --workspace --all-features --no-deps`.
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
Real Telegram delivery and the wider concurrency, revocation, crash,
update/recovery and browser matrix are still required before completing MVP-05.

Record source repository commits, build/release/instance/revision IDs, route
and mailbox IDs, state-volume and fenced-lease IDs, dispatch order and
state-access outcomes, authorization snapshots, secret versions and leases,
HTTPS egress usage records, Git target/result commits, update and recovery
IDs, denial evidence, test counts, screenshots, and exact verification
commands.
