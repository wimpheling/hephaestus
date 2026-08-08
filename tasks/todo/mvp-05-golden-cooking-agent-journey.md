# MVP 05: Golden cooking-agent journey

Owner: unassigned

## Outcome

Prove the “own the loop” product end to end with a small family cooking agent
implemented as ordinary released software.

Two authorized family members communicate through released Telegram gateway
code. The cooking agent owns its model loop and persistent recipe memory,
updates a static cooking blog through controlled Git publication, uses model
and Telegram APIs through declared destination-bound HTTPS egress without
receiving provider credentials, survives a
release update, and exposes exact release, state-volume, lease, authorization,
trigger, capability-use, and result provenance.

This is the acceptance task for MVP 01 through MVP 04, not a source of new
platform-specific Telegram or cooking abstractions.

## Locked decisions

| Area | Decision |
| --- | --- |
| Application ownership | The released gateway and cooking-agent repositories own Telegram semantics, user mapping, prompts, memory schema, model loop, retry/idempotency policy, blog generation, and user experience. |
| Users | Exactly two configured family identities are authorized in the golden path; an unrecognized identity is rejected by released policy before it reaches project authority. |
| Gateway boundary | The public gateway is a separate principal with a synchronous HTTP handler under the shared Caddy `/gateway/` namespace. It may publish only to the cooking-agent mailbox and has no repository or cooking-agent state authority. |
| Agent authority | The cooking agent may consume its mailbox, use its state, call explicitly bound HTTPS destinations through placeholder substitution, and read/propose changes to one blog repository. |
| Publication | Canonical Git mutation remains a host-side controlled operation with exact target commit and authorization provenance. |
| Credentials | Model, outbound-provider, and Telegram inbound verification credentials remain brokered and unavailable as raw guest values. The gateway receives only a non-secret Telegram placeholder: MVP 04 rewrites a valid inbound header to that placeholder, and substitutes the real value only on the authorized outbound HTTPS request. |
| State | Recipe memory is durable application state in the instance volume; the process may stop and reconstruct from state and mailbox events. |
| Testability | Deterministic fake Telegram and model upstreams are the required automated path. A real-provider smoke test is optional and must not weaken credential controls. |

## Dependencies

- [`mvp-01-agent-principals-capabilities-and-runtime-authority.md`](mvp-01-agent-principals-capabilities-and-runtime-authority.md)
- [`mvp-01.2-replace-controlled-result-publication-with-runtime-git.md`](mvp-01.2-replace-controlled-result-publication-with-runtime-git.md)
- [`mvp-02-durable-agent-mailboxes-and-stateful-dispatch.md`](mvp-02-durable-agent-mailboxes-and-stateful-dispatch.md)
- [`mvp-03-event-ingress-and-caddy-routing.md`](mvp-03-event-ingress-and-caddy-routing.md)
- [`mvp-04-brokered-model-and-outbound-capabilities.md`](mvp-04-brokered-model-and-outbound-capabilities.md)
- [`define-own-the-loop-agent-platform.md`](define-own-the-loop-agent-platform.md)

## Non-goals

This task does not produce a universal assistant, beginner distribution,
catalog, Operator/Admin Agent, Project Agent, web search, browser use,
WebSockets, streaming services, general scheduling, real-provider availability
guarantees, or production marketing material.

## Implementation checklist

- [ ] **1. Specify the complete acceptance fixture**
  - [ ] **Define released application behavior**
    - [ ] Specify the gateway request validation, Telegram update parsing,
      stable application deduplication, two-user mapping, normalization,
      HTTP acknowledgement/status contract, and cooking-mailbox publication
      contract. Specify the exact Telegram brokered placeholder slot, inbound
      header rule, rotation behavior, and rejected-request responses.
    - [ ] Specify the cooking agent's model loop, bounded context, recipe
      SQLite schema, transaction and idempotency policy, blog rendering, and
      outbound response behavior.
    - [ ] Specify the static cooking-blog repository layout, build/check
      command, result proposal, approval, and controlled publication flow.
    - [ ] Specify stable release configuration for parameters, capability
      requirements, secret slots, state, resource bounds, network ceiling,
      runtime commands, and update hook.
  - [ ] **Define allowed and denied authority**
    - [ ] Record the exact gateway and cooking-agent capability declarations,
      concrete bindings, grants, HTTPS destinations/substitution rules, repository,
      mailbox, route, secrets, and state volume.
    - [ ] Record explicit denials for other users, projects, repositories,
      mailboxes, routes, secrets, undeclared API destinations, authorization changes,
      direct canonical Git writes, and Caddy administration.
    - [ ] Define acceptance assertions for every allowed and denied operation.

- [ ] **2. Build and publish the reference releases**
  - [ ] Create small reviewable source repositories for the gateway and
    cooking agent without privileged framework dependencies.
  - [ ] Build both in isolated build guests and publish immutable releases with
    exact source, build, artifact-manifest, runtime-policy, capability, and
    secret-slot provenance.
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
  - [ ] Create and bind model-API, Telegram-API, and Telegram-verification
    secrets without exposing values to the binding user or either guest. Bind
    their exact placeholder, destination, and gateway-route substitution rules.
  - [ ] Bind the cooking agent to one exact blog repository/ref and bounded
    HTTPS destination and placeholder-substitution bindings.
  - [ ] Record the exact installation, revision, attachment, route,
    authorization snapshot, state volume, fenced lease, dispatch order, and
    secret binding fixture IDs.

- [ ] **4. Exercise normal operation**
  - [ ] Send simultaneous fake Telegram requests from both authorized users
    through Caddy and receive the handler's specified bounded HTTP responses.
  - [ ] Send valid, missing, invalid, and rotated-secret Telegram requests and
    verify that the authorized inbound header is rewritten to the placeholder,
    gateway repository code returns the specified responses, and no cooking
    agent, repository, HTTPS egress, or state authority is used before rejection.
  - [ ] Verify the gateway normalizes and publishes only the expected bounded
    events to the cooking mailbox.
  - [ ] Verify stateful cooking runs serialize, call declared model and Telegram APIs
    through destination-bound placeholder substitution, update recipe memory
    transactionally, and handle ordinary API responses in repository code.
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
  - [ ] Rotate the Telegram and model credentials and prove later operations
    use the new exact versions while earlier run provenance remains intact.
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
    application-API and Telegram verification-secret sentinels.

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

Record source repository commits, build/release/instance/revision IDs, route
and mailbox IDs, state-volume and fenced-lease IDs, dispatch order and
state-access outcomes, authorization snapshots, secret versions and leases,
HTTPS egress usage records, Git target/result commits, update and recovery
IDs, denial evidence, test counts, screenshots, and exact verification
commands.
