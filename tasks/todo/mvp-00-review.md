# MVP 00: Review of MVP plan coherence and delivery risks

Owner: unassigned

## Purpose

This review records cross-plan gaps discovered in the MVP 01 through MVP 06
planning set. It is a planning and acceptance-criteria document: before an
affected MVP is considered ready to implement, its owning plan should resolve
the relevant decision and add the specified evidence to its completion gate.

## Summary

The current plans provide a strong capability-first foundation. All eleven
review findings are now either resolved in the affected plans or explicitly
deferred outside this MVP's scope, including Git-backed retention and fork
privacy semantics.

## 1. Make runtime credential issuance crash-idempotent

Decision recorded: each runtime credential has a stable issuance generation and
a temporary encrypted handoff envelope that only trusted host bootstrap code
can read. PostgreSQL continues to store only the credential hash. The guest
acknowledges receipt of the exact generation before the envelope is deleted;
retries re-deliver the same credential and duplicate acknowledgement is
idempotent.

If acknowledgement does not arrive before its deadline, Hephaestus revokes the
session, terminates the guest, and deletes the envelope. It never mints a
replacement credential for an active session; a replacement requires a new
session after revocation. Failure-injection coverage must span credential
generation, session/hash commit, envelope persistence, bootstrap delivery,
guest acknowledgement, envelope deletion, and guest start.

Affected plan: [MVP 01](mvp-01-agent-principals-capabilities-and-runtime-authority.md).

## 2. Assign authoritative ownership of gateway identity and authority

Decision recorded: gateways are repository-declared workloads at the same level
as agents, and are not agent-instance subtypes. A declaration requests bounded
routes and maps them to a versioned, stateless HTTP request handler. Hephaestus
invokes the handler in a short-lived VM and relays its bounded HTTP response.
The gateway has its own immutable revision, per-invocation runtime session,
audit identity, and explicitly granted capabilities; it has no mailbox, agent
state, or repository authority unless a specific capability grants one.

A host-side `GatewayProvider` adapter reconciles desired routes and translates
provider-specific requests and responses to/from the canonical HTTP contract.
MVP 03 implements only `LocalCaddyGatewayProvider`, which uses the existing
shared Caddy server and its reserved `/gateway/` namespace. Caddy terminates
HTTPS and the private VM invocation is HTTP; no gateway certificate policy is
available. Cloudflare and AWS adapters remain future implementations.

Document one owner for gateway snapshots, runtime sessions, RLS identity,
OpenFGA relations, revision lifecycle, and audit attribution. Add cross-kind
forgery tests so a gateway cannot impersonate an agent instance or obtain its
authority, plus reconciliation tests proving that gateway routes share the
existing Caddy deployment without affecting platform-owned routes.

Affected plans: [MVP 01](mvp-01-agent-principals-capabilities-and-runtime-authority.md),
[MVP 02](mvp-02-durable-agent-mailboxes-and-stateful-dispatch.md), and
[MVP 03](mvp-03-event-ingress-and-caddy-routing.md).

## 3. Choose inbound Telegram credential handling

Decision recorded: a public gateway never receives its Telegram verification
secret. The authorized ingress route compares the real inbound header and
rewrites a valid value to the binding's non-secret placeholder before VM
delivery. The released gateway repository compares that placeholder and returns
its own HTTP responses; the same placeholder is replaced with the real secret
only on its exact authorized outbound HTTPS destination.

Test valid, missing, invalid, replayed, and rotated-secret requests; prove no
Telegram verification secret appears in PostgreSQL, NATS, routes, logs, traces,
browser payloads, screenshots, VM environment, files, or process arguments.

Affected plans: [MVP 03](mvp-03-event-ingress-and-caddy-routing.md),
[MVP 04](mvp-04-brokered-model-and-outbound-capabilities.md), and
[MVP 05](mvp-05-golden-cooking-agent-journey.md).

## 4. Defer session-history integrity to a focused follow-up

Decision recorded: session-history integrity is repository/release-owned, not a
Hephaestus receive-time protocol. The focused follow-up is
[Draft: Repository-owned Git session protocol](draft-git-backed-session-history-integrity.md).
Hephaestus supplies generic scoped Git access and authenticated receive
attribution; MVP 06's selected release owns its session layout and rules.

## 5. Delegate concurrent session writes to the repository/release

Decision recorded: turn ordering, concurrent writers, conflict handling,
retry, and causal semantics are repository/release responsibilities. Hephaestus
enforces only generic repository/ref/path and Git-transition capabilities. The
selected chat release owns these rules in
[Draft: Repository-owned Git session protocol](draft-git-backed-session-history-integrity.md).

## 6. Reconcile gateway revision cutover semantics

Decision recorded: MVP 03 performs synchronous HTTP invocation. The
GatewayDispatcher resolves the enabled route and exact gateway revision at the
start of each request, invokes that revision, and drains or cancels in-flight
requests during cutover. There is no accepted-but-later gateway dispatch.

If gateway code subsequently publishes to an agent mailbox, that is a separate
operation: MVP 02 independently binds the target agent revision when the
mailbox event becomes eligible for dispatch. Record both gateway invocation and
mailbox-delivery provenance, but do not treat them as one gateway revision
binding. Test route updates during in-flight requests and gateway publication
across an agent revision update.

## 7. Update the umbrella roadmap and dependency DAG

Decision recorded: the umbrella now contains the authoritative dependency DAG
for MVP 01, 01.1, 01.2, 02, 03, 04, 05, and 06. MVP 06 is an active journey
that depends only on runtime Git and MVP 04's generic destination-bound HTTPS
egress. The roadmap distinguishes runtime Git from proposal-mode publication.

Affected plan: [Define the own-the-loop agent platform](define-own-the-loop-agent-platform.md).

## 8. Decouple chat prerequisites and include proposal-mode migration

Decision recorded: MVP 04 is generic destination-bound HTTPS egress with
placeholder secret substitution, not model brokering or an outbound adapter.
MVP 06 uses it for its model API and MVP 05 uses it for its model and Telegram
APIs; only MVP 05 requires MVP 03 for its public gateway. MVP 05 now explicitly
depends on MVP 01.2 for the selected runtime-Git/proposal-publication migration.

Affected plans: [MVP 01.2](mvp-01.2-replace-controlled-result-publication-with-runtime-git.md),
[MVP 04](mvp-04-brokered-model-and-outbound-capabilities.md),
[MVP 05](mvp-05-golden-cooking-agent-journey.md), and
[MVP 06](mvp-06-git-backed-session-chat-journey.md).

## 9. Defer LLM budget accounting

Decision recorded: Hephaestus does not manage LLM token or monetary budgets in
this MVP. MVP 04 provides generic HTTPS egress and secret substitution, not
model invocation or provider spend accounting. Provider-side quotas and billing
remain outside the platform contract until Hephaestus adopts native LLM spend
management.

Affected plan: [MVP 04](mvp-04-brokered-model-and-outbound-capabilities.md).

## 10. Give newer MVPs concrete completion and verification contracts

Decision recorded: MVP 01.1, MVP 01.2, and MVP 06 now each include `Verify and
document` and `Completion evidence` sections. They require the repository Rust
checks (`cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features`,
`cargo test --workspace --all-features`, and
`cargo doc --workspace --all-features --no-deps`), plus `git diff --check` and
`cargo dev quality` at repository handoff. Each plan also names the applicable
real-Git, PostgreSQL, runtime, browser, egress, security, and migration
evidence, and requires recorded results or an explicit justified exclusion.

Affected plans: [MVP 01.1](mvp-01.1-git-capabilities-and-developer-personal-access-tokens.md),
[MVP 01.2](mvp-01.2-replace-controlled-result-publication-with-runtime-git.md),
and [MVP 06](mvp-06-git-backed-session-chat-journey.md).

## 11. Decide Git-backed chat retention and fork privacy semantics

Decision recorded: Git retention, deletion/erasure, and fork-privacy semantics
are outside this MVP. MVP 06 must not promise erasure from Git object history
or independently cloned/forked repositories. Encryption and key-destruction
policy, attachment/content capability authorization, and any user-facing
retention promises are deferred to a later scoped design.

Affected plan: [MVP 06](mvp-06-git-backed-session-chat-journey.md).

## Exit criteria

- [ ] Each affected MVP incorporates or explicitly rejects its findings with a
  recorded rationale.
- [ ] The roadmap dependency DAG reflects the resulting decisions.
- [ ] Cross-MVP integration and failure tests cover the selected protocols.
- [ ] The implementation handoff records the required verification evidence.
