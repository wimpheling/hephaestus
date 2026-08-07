# MVP 00: Review of MVP plan coherence and delivery risks

Owner: unassigned

## Purpose

This review records cross-plan gaps discovered in the MVP 01 through MVP 06
planning set. It is a planning and acceptance-criteria document: before an
affected MVP is considered ready to implement, its owning plan should resolve
the relevant decision and add the specified evidence to its completion gate.

## Summary

The current plans provide a strong capability-first foundation, but five issues
must be resolved before the MVPs can compose safely: crash-safe runtime
credential issuance, an authoritative gateway identity model, brokered inbound
Telegram verification, enforceable append-only session history, and concurrent
session write semantics. The remaining findings concern cutover semantics,
roadmap dependencies, budget accounting, completion evidence, and Git-backed
retention expectations.

## 1. Make runtime credential issuance crash-idempotent

MVP 01 requires a single opaque credential, hash-only persistence,
bootstrap-only delivery, and idempotent redelivery. A crash after committing
the verifier but before mounting or delivering the bootstrap value loses the
only plaintext. Retrying cannot reproduce that value, while minting a second
credential violates the one-credential invariant.

Specify an issuance-generation and bootstrap acknowledgement protocol, or an
encrypted transient envelope with a bounded recovery path. Define when an
abandoned verifier becomes invalid and who performs that invalidation. Add
failure-injection coverage at generation, hash commit, mount, and guest-start
boundaries, including retry and recovery cases.

Affected plan: [MVP 01](mvp-01-agent-principals-capabilities-and-runtime-authority.md).

## 2. Assign authoritative ownership of gateway identity and authority

MVP 01 excludes gateways, MVP 02 treats a mailbox as belonging to an agent
instance, and MVP 03 introduces a distinct gateway principal with its own
mailbox and runtime credential. The plans must settle whether a gateway is an
agent-instance subtype or a separate principal kind.

Document one owner for gateway snapshots, runtime sessions, RLS identity,
OpenFGA relations, mailbox schema, revision lifecycle, and audit attribution.
Add cross-kind forgery tests so a gateway cannot impersonate an agent instance
or obtain its authority.

Affected plans: [MVP 01](mvp-01-agent-principals-capabilities-and-runtime-authority.md),
[MVP 02](mvp-02-durable-agent-mailboxes-and-stateful-dispatch.md), and
[MVP 03](mvp-03-event-ingress-and-caddy-routing.md).

## 3. Define brokered inbound Telegram authentication

MVP 03 says gateway code owns signatures and tokens, while MVP 05 states that
Telegram credentials never reach either guest. MVP 04 defines outbound delivery
but no corresponding inbound verification path. Under those boundaries, the
gateway cannot verify Telegram requests without an explicitly brokered service.

Add a brokered inbound verification operation that performs constant-time
token, HMAC, or signature verification and returns only a verdict plus a
sanitized request. Alternatively, explicitly permit raw credential delivery and
revise the threat model. Test invalid, missing, and replayed signatures, and
prove that sentinel credentials do not leak into guest-visible state.

Affected plans: [MVP 03](mvp-03-event-ingress-and-caddy-routing.md),
[MVP 04](mvp-04-brokered-model-and-outbound-capabilities.md), and
[MVP 05](mvp-05-golden-cooking-agent-journey.md).

## 4. Enforce append-only session history beyond Git fast-forward rules

The session contract declares user and agent records append-only, but a
fast-forward-only push with changed-path globs still permits a new commit to
edit or delete an earlier record. Git author metadata is also not a trustworthy
actor identity.

Add receive-time protocol validation and server-side attribution, or use
write-once actor namespaces that the Git capability layer enforces. Validate
history-record mutation, forged actor and correlation identifiers, duplicate
record IDs, and unauthorized content references. Do not treat ref movement or
Git author fields as proof of append-only or actor integrity.

Affected plans: [MVP 01.2](mvp-01.2-replace-controlled-result-publication-with-runtime-git.md)
and [MVP 06](mvp-06-git-backed-session-chat-journey.md).

## 5. Specify a satisfiable concurrent session push protocol

Runtime Git may fast-forward only from the exact triggering commit, while each
UI message and each agent response advances the same session branch. A second
message accepted while a run is in progress makes the first response non-fast-
forward, so the proposed rule cannot handle ordinary concurrent interaction.

Choose one model: a per-session turn gate with queued inputs, per-turn response
refs with a host-side merge, or a validated compare-and-swap/rebase protocol.
Define retry behavior and causal ordering. Add integration tests for two
simultaneous tabs, interleaved user input, response retry, and stale-run output.

Affected plans: [MVP 01.2](mvp-01.2-replace-controlled-result-publication-with-runtime-git.md)
and [MVP 06](mvp-06-git-backed-session-chat-journey.md).

## 6. Reconcile gateway revision cutover semantics

MVP 02 binds queued work to the revision active when work becomes eligible for
dispatch. MVP 03 persists a gateway revision on its route, starts that exact
revision, and says accepted requests retain it. These rules differ when a route
changes between request acceptance and dispatch.

Decide whether ingress pins the parser revision at acceptance or whether work
uses the revision active at dispatch. Persist both a route-binding revision and
an execution revision when they can differ. Test acceptance before a route
update followed by dispatch after the update, as well as retries across the
cutover.

Affected plans: [MVP 02](mvp-02-durable-agent-mailboxes-and-stateful-dispatch.md)
and [MVP 03](mvp-03-event-ingress-and-caddy-routing.md).

## 7. Update the umbrella roadmap and dependency DAG

The umbrella roadmap still describes five child tasks and defers interactive
sessions, despite the addition of MVP 01.1, MVP 01.2, and MVP 06. Its
controlled-publication language also needs reconciliation with the new runtime
Git and proposal-mode approach.

Replace the stale list with an explicit dependency DAG covering all current MVP
documents. Either promote MVP 06 into the MVP scope or clearly label it as
post-MVP. Align the publication terminology and the migration path so there is
one source of truth for execution modes.

Affected plan: [Define the own-the-loop agent platform](define-own-the-loop-agent-platform.md).

## 8. Decouple chat prerequisites and include proposal-mode migration

MVP 04 bundles model brokering and Telegram outbound delivery, causing MVP 06's
model dependency to transitively require public ingress despite MVP 06's
non-goals. Separately, MVP 05 relies on host-side proposal publication but does
not depend on MVP 01.2, which makes proposal mode and its migration explicit.

Split model brokering from Telegram outbound delivery, or phase MVP 04 so chat
can depend on the model capability alone. Add MVP 01.2 as an MVP 05 dependency,
or explicitly order and test the proposal-publication migration.

Affected plans: [MVP 01.2](mvp-01.2-replace-controlled-result-publication-with-runtime-git.md),
[MVP 04](mvp-04-brokered-model-and-outbound-capabilities.md),
[MVP 05](mvp-05-golden-cooking-agent-journey.md), and
[MVP 06](mvp-06-git-backed-session-chat-journey.md).

## 9. Add authoritative concurrent model-budget accounting

MVP 04 requires cumulative run and project budget enforcement plus idempotency,
but provider acceptance can be uncertain. Without an authoritative reservation
ledger, concurrent calls can overspend and retries can be charged twice.

Reserve the worst-case budget before invoking the provider, under a unique
scoped idempotency key. Define finalization, release, and uncertain-provider
states, including an operator reconciliation path. Add concurrent-call and
failure-injection tests that demonstrate no overspend or double charge.

Affected plan: [MVP 04](mvp-04-brokered-model-and-outbound-capabilities.md).

## 10. Give newer MVPs concrete completion and verification contracts

MVP 01.1 and MVP 06 end at non-goals, and MVP 01.2 has only a broad quality
gate. None gives a complete evidence contract for implementation readiness or
handoff.

Add completion, verification, and documentation sections to each plan. At a
minimum, require the repository Rust checks: `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features`,
`cargo test --workspace --all-features`, and
`cargo doc --workspace --all-features --no-deps`. Add applicable real-Git,
PostgreSQL, libkrun, Playwright, and security suites; require `git diff --check`
and `cargo dev quality` for the repository handoff gate. Record the resulting
evidence in the completed task.

Affected plans: [MVP 01.1](mvp-01.1-git-capabilities-and-developer-personal-access-tokens.md),
[MVP 01.2](mvp-01.2-replace-controlled-result-publication-with-runtime-git.md),
and [MVP 06](mvp-06-git-backed-session-chat-journey.md).

## 11. Decide Git-backed chat retention and fork privacy semantics

MVP 06 makes sessions durable and forkable in Git while also promising
retention/tombstone behavior and safe forks. A tombstone does not erase prior
Git objects or independently cloned forks. It also references immutable content
without establishing an attachment/content-capability path, while generic
uploads remain out of scope.

Choose either explicit non-erasure semantics with clear UI warnings, or
encryption with key destruction and a documented fork-key policy. For v1,
either exclude attachments/content references or add the necessary content
capability dependency and authorization model.

Affected plan: [MVP 06](mvp-06-git-backed-session-chat-journey.md).

## Exit criteria

- [ ] Each affected MVP incorporates or explicitly rejects its findings with a
  recorded rationale.
- [ ] The roadmap dependency DAG reflects the resulting decisions.
- [ ] Cross-MVP integration and failure tests cover the selected protocols.
- [ ] The implementation handoff records the required verification evidence.
