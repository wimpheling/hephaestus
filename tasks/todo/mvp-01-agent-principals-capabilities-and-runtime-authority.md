# MVP 01: Agent principals, capabilities, and runtime authority

Owner: unassigned

## Outcome

Make released agent instances and gateway instances first-class, narrowly
authorized runtime principals.

A release declares symbolic capability requirements, an authorized project
binds those requirements to exact resources, and dispatch mints a short-lived
opaque credential for one exact run. Every privileged call is constrained by
the credential ceiling, checked against live OpenFGA/Mélange authorization,
executed through PostgreSQL RLS, and recorded in structured audit provenance.

This task establishes the security foundation for every later MVP task. It
extends the reusable-release and secret-runtime work without introducing a
second authorization or credential system.

## Locked decisions

| Area | Decision |
| --- | --- |
| Authority | The canonical OpenFGA model, Mélange-generated SQL, authoritative domain relationships, and PostgreSQL RLS remain the authorization authority. |
| Declarations | A release capability declaration requests a symbolic slot and grants no authority. |
| Bindings | An immutable instance revision resolves each required slot to an exact resource and permitted operation set. |
| Subjects | Users, agent instances, gateway instances, runs, and trusted workers are distinct subjects. Project membership never gives an agent implicit maintainer authority. |
| Runtime credentials | MVP runtime credentials are opaque, random, short-lived, server-side records stored only as hashes. Queued work never contains a previously minted bearer token. |
| Effective permission | A request must fit the immutable credential ceiling and pass live authorization. Either check may deny it. |
| Database access | Agent-facing APIs use a non-`BYPASSRLS` role with typed transaction-local subject context. |
| Initial scope | The initial semantic capabilities cover inbox consumption/publication, model invocation, brokered outbound responses, instance state use, and bounded repository read/propose operations. |
| Secrets | Secret versions, bindings, leases, and brokered credential application remain owned by `manage-delegate-and-deliver-secrets.md`; this task integrates their runtime authority into the general principal model. |

## Dependencies

- [`reusable-agent-releases-and-instances.md`](../done/reusable-agent-releases-and-instances.md)
- [`manage-delegate-and-deliver-secrets.md`](../done/manage-delegate-and-deliver-secrets.md)
- [`define-own-the-loop-agent-platform.md`](define-own-the-loop-agent-platform.md)

## Non-goals

This task does not add an Operator/Admin Agent, general human-to-agent
delegation, one-shot management approvals, long-lived service credential
renewal, public ingress routing, a universal tool protocol, or provider-specific
agent-loop behavior.

## Implementation checklist

- [ ] **1. Finalize the MVP capability contract**
  - [ ] **Define stable domain vocabulary**
    - [ ] Add provider-neutral identifiers and bounded values for capability
      requirements, bindings, grants, authorization snapshots, and runtime
      credentials where equivalent types do not already exist.
    - [ ] Define the initial resource kinds and semantic operations without
      falling back to broad filesystem-like permissions.
    - [ ] Define deterministic hashes and idempotency identities for
      declarations, bindings, snapshots, credentials, grants, and revocations.
    - [ ] Add focused parsing, normalization, serialization, malformed-input,
      and deterministic-hash tests.
  - [ ] **Lock the release and revision extension points**
    - [ ] Extend the versioned release configuration with bounded symbolic
      capability requirements and normalized operation sets.
    - [ ] Bind normalized requirements and their hash into the immutable
      release agent.
    - [ ] Bind symbolic slots to exact resources and restricted operations in
      immutable instance revisions.
    - [ ] Reject missing bindings, unknown resource kinds, operation
      broadening, cross-project resources, and consumer attempts to alter
      release-owned declarations with stable diagnostics.
    - [ ] Preserve compatibility for releases that declare no general
      capabilities.

- [ ] **2. Add authoritative capability and snapshot records**
  - [ ] **Persist lifecycle state**
    - [ ] Add normalized requirement, binding, grant, revocation, immutable
      authorization-snapshot, and runtime-credential records with complete
      foreign keys and tombstone history.
    - [ ] Ensure revisions and historical runs continue resolving exact
      bindings after live grants are revoked.
    - [ ] Add compare-and-swap and uniqueness constraints preventing stale
      binding activation, duplicate grants, or conflicting live credentials.
    - [ ] Add real-PostgreSQL tests for immutability, tenant boundaries,
      revocation, tombstones, and concurrent creation.
  - [ ] **Integrate existing secret authority**
    - [ ] Reuse secret binding and lease identities in the authorization
      snapshot rather than copying secret policy into generic capability
      records.
    - [ ] Ensure `secret.use_brokered` and `secret.receive_raw` remain distinct
      and neither implies secret metadata or management authority.
    - [ ] Test mixed revisions containing repository, inbox, model, outbound,
      state, and secret-backed bindings.

- [ ] **3. Extend OpenFGA, Mélange, and RLS**
  - [ ] **Model runtime principals**
    - [ ] Model `agent_instance` and `gateway_instance` as authorization
      subjects that receive only explicit resource relations.
    - [ ] Model the minimum run/runtime objects needed for exact credential
      ceilings and live permission checks.
    - [ ] Define who may bind, grant, revoke, inspect, and execute every MVP
      capability.
    - [ ] Prove that owning or maintaining a project does not automatically
      make its agents project maintainers or secret managers.
  - [ ] **Generate and enforce database authorization**
    - [ ] Derive authoritative `melange_tuples` from capability resources,
      bindings, grants, instances, gateways, and runtime records.
    - [ ] Regenerate and commit the specialized Mélange SQL using the
      repository-pinned CLI.
    - [ ] Apply forced RLS to all new capability, snapshot, credential, and
      audit tables.
    - [ ] Extend OpenFGA/Mélange parity fixtures, unknown-object denial,
      normal-role non-bypass tests, and generated-migration drift checks.

- [ ] **4. Generalize request identity**
  - [ ] **Add typed transaction-local context**
    - [ ] Support subject type and ID, request ID, run or session ID, mediator
      agent ID, and delegation ID without trusting caller-supplied database
      settings.
    - [ ] Define and implement distinct flows for autonomous agents, gateways,
      authenticated users, and trusted mechanical workers.
    - [ ] Ensure runtime requests cannot select the trusted worker role or
      write an arbitrary effective subject.
    - [ ] Add real-PostgreSQL tests showing RLS behavior for every MVP subject
      type and rejection of forged or incomplete context.

- [ ] **5. Issue and check runtime credentials**
  - [ ] **Mint at dispatch**
    - [ ] Reauthorize the exact instance, revision, run, attachment, and live
      binding set immediately before dispatch.
    - [ ] Persist an immutable authorization snapshot and mint one opaque
      short-lived credential bound to its exact run and capability ceiling.
    - [ ] Store only a credential hash and keep bearer material out of
      PostgreSQL plaintext columns, NATS, logs, traces, and queued work.
    - [ ] Make retries and NATS redelivery reuse the logical runtime identity
      without creating conflicting active credentials.
  - [ ] **Validate every privileged call**
    - [ ] Authenticate the credential and validate expiry, revocation, run,
      instance, revision, lease, action, and exact target resource.
    - [ ] Apply the immutable ceiling before a live Mélange permission check
      and RLS-constrained transaction.
    - [ ] Deny new calls immediately after a binding, grant, credential, or
      underlying resource authorization is revoked.
    - [ ] Define cancellation or lease withdrawal behavior for capabilities
      already materialized as guest mounts.
    - [ ] Add tests for theft across runs, instances, revisions, gateways,
      resources, operations, expiry, and revocation races.

- [ ] **6. Audit and observe runtime authority**
  - [ ] **Record complete decisions**
    - [ ] Record requester, mediator, runtime principal, run, snapshot,
      permission, resource, authorization-model version, request ID, decision,
      and result for every privileged API call.
    - [ ] Redact bearer material, secret references marked sensitive, parameter
      values marked sensitive, and provider credential material.
    - [ ] Add metrics for credential issuance, denials, expiry, revocation
      latency, and live permission failures using opaque identifiers.
    - [ ] Add inspection tooling that explains a denial or effective ceiling
      without revealing protected values.

- [ ] **7. Verify and document**
  - [ ] Document the declaration-to-binding-to-snapshot-to-runtime lifecycle
    and the distinction between credential ceilings and live authorization.
  - [ ] Document each runtime subject, database role, initial semantic
    capability, revocation guarantee, and unavoidable mounted-resource limit.
  - [ ] Run `cargo fmt --all -- --check`.
  - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
  - [ ] Run `cargo test --workspace --all-features`.
  - [ ] Run `cargo doc --workspace --all-features --no-deps`.
  - [ ] Run the real-PostgreSQL authorization suites, Mélange drift detection,
    `melange doctor`, and OpenFGA compatibility fixtures.
  - [ ] Run `git diff --check`.

## Completion evidence

Record accepted vocabulary and schema versions, migration IDs, authorization
model and generated-SQL hashes, test counts, golden runtime principal and
snapshot IDs, denial/revocation evidence, and deliberate follow-up tasks.
