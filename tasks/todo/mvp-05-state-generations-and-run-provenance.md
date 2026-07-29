# MVP 05: State generations and exact run provenance

Owner: unassigned

## Outcome

Make the behavior of a mutable stateful agent inspectable without pretending
that every run has a complete reproducible state snapshot.

Each stateful run records an exact pre-run volume generation, an exact
post-run generation when it commits state changes, the immutable release and
instance revision, trigger, authorization snapshot, runtime principal,
capability uses, and result. Crashes and uncertain outcomes remain explicit,
and operator inspection can reconstruct the complete platform-controlled
chain.

## Locked decisions

| Area | Decision |
| --- | --- |
| Distinctions | A generation ID, integrity hash, checkpoint, backup, and VM snapshot are different concepts and must not be presented as interchangeable. |
| MVP evidence | Every stateful run records pre-run generation. Successful state commit records a new post-run generation. |
| Serialization | Generation transitions occur under the instance volume's exclusive fenced lease and cannot race another stateful run or update hook. |
| Reproducibility | A generation proves ordering and exact platform identity; it does not claim that mutable state bytes are retained or reproducible. |
| Privacy | State bytes remain private instance data. Provenance stores opaque IDs and bounded metadata, not arbitrary state contents. |
| Runtime snapshots | VM/process snapshots are not canonical agent state and are outside this MVP. |
| Authorization | Provenance refers to the immutable authorization snapshot and records live authorization decisions made during execution. |
| History | Release, revision, attachment, trigger, state generations, capability uses, and result records needed by historical runs are tombstoned rather than hard-deleted. |

## Dependencies

- [`mvp-01-agent-principals-capabilities-and-runtime-authority.md`](mvp-01-agent-principals-capabilities-and-runtime-authority.md)
- [`mvp-02-durable-agent-mailboxes-and-stateful-dispatch.md`](mvp-02-durable-agent-mailboxes-and-stateful-dispatch.md)
- [`reusable-agent-releases-and-instances.md`](../done/reusable-agent-releases-and-instances.md)
- [`define-own-the-loop-agent-platform.md`](define-own-the-loop-agent-platform.md)

## Non-goals

This task does not add full per-run volume snapshots, process checkpointing,
snapshot resume, state diffing, arbitrary memory inspection, state sharding,
session volumes, host-managed migration rollback, or a general backup product.

## Implementation checklist

- [ ] **1. Define state-generation contracts**
  - [ ] **Add stable domain types and transitions**
    - [ ] Add provider-neutral generation, transition, integrity-evidence,
      state-operation, and uncertainty identifiers and bounded values.
    - [ ] Define initial, unchanged, committed, update-migrated, recovery,
      uncertain, and retired generation relationships.
    - [ ] Define deterministic identities for generation creation, run
      transitions, update transitions, retries, and recovery.
    - [ ] Define which metadata may be retained without reading or serializing
      agent-owned state contents.
    - [ ] Add domain tests for validation, transition invariants,
      deterministic identities, serialization, and malformed input.

- [ ] **2. Persist immutable generation history**
  - [ ] **Add PostgreSQL records and constraints**
    - [ ] Persist instance volume generations, predecessor relationships,
      creating run/update/recovery operation, fenced lease token, timestamps,
      optional integrity evidence, and explicit uncertainty.
    - [ ] Require one current generation per instance state volume and use
      compare-and-swap activation under the active fenced lease.
    - [ ] Prevent cross-instance generation links, generation reuse by stale
      leases, mutation after activation, and hard deletion while referenced.
    - [ ] Apply forced RLS and exact instance inspect and recovery permissions.
    - [ ] Add real-PostgreSQL tests for concurrent CAS, stale lease denial,
      immutability, tenant isolation, RLS, tombstones, and recovery transitions.

- [ ] **3. Bind generations to normal runs and updates**
  - [ ] **Capture pre-run state**
    - [ ] Resolve and persist the exact current state generation in the same
      durable dispatch boundary that binds the run revision and acquires the
      state lease.
    - [ ] Include the pre-run generation ID in host-generated immutable
      runtime context without exposing host paths or mutable generation
      metadata.
    - [ ] Refuse launch when the selected generation, volume, instance, lease,
      or revision relationship is stale or inconsistent.
  - [ ] **Commit post-run state**
    - [ ] Create and activate a successor generation only after the successful
      runtime result boundary and required filesystem durability operations.
    - [ ] Record an unchanged disposition when a stateless run or verified
      read-only state use produces no state transition.
    - [ ] Mark state outcome uncertain when guest, provider, filesystem, lease,
      or host failure prevents an honest commit determination.
    - [ ] Integrate update-hook success, explicit rollback, abnormal failure,
      and authorized recovery with generation history without claiming
      host-managed rollback.
    - [ ] Add real-libkrun tests for success, no change, explicit update
      rollback, timeout, signal, VM loss, host crash, and stale completion.

- [ ] **4. Complete exact run provenance**
  - [ ] **Persist the platform-controlled chain**
    - [ ] Bind each run to its release, artifact manifest, instance revision,
      attachment, target repository/ref/commit, mailbox or repository trigger,
      authorization snapshot, runtime credential identity, and pre/post state
      generations.
    - [ ] Record every privileged capability use with its exact binding,
      resource, permission, live authorization decision, request ID, and
      result.
    - [ ] Record result publication, outbound response references, model usage,
      secret version/lease identities, and final run disposition without
      protected payloads or values.
    - [ ] Preserve historical resolution after attachments, grants, releases,
      routes, secrets, or instances are revoked or tombstoned.
    - [ ] Add tests that reconstruct the complete provenance chain after
      instance revision updates, release revocation, secret rotation,
      attachment removal, and state transitions.

- [ ] **5. Add retention, privacy, and inspection**
  - [ ] Define retention separately for generation metadata, integrity
    evidence, logs, mailbox payloads, runtime results, secret provenance, and
    future state backups.
  - [ ] Ensure inspection never mounts or reads state bytes merely to display
    generation provenance.
  - [ ] Add authorized read-only operator and project views for the run chain,
    current generation, predecessor history, uncertain transitions, and
    recovery requirements.
  - [ ] Redact sensitive parameters, prompts, message bodies, state paths,
    secret values, provider payloads, and unauthorized resource metadata.
  - [ ] Reauthorize live provenance subscriptions before publishing updates.

- [ ] **6. Reconcile and observe**
  - [ ] Reconcile incomplete generation transitions after dispatch, guest
    start, result receipt, durability operations, generation activation,
    guest destruction, and lease release.
  - [ ] Never advance the current generation from an expired fenced lease or
    infer success solely from guest termination.
  - [ ] Trace run, instance, revision, volume, lease, pre/post generation,
    authorization snapshot, trigger, result, and recovery IDs.
  - [ ] Measure generation transition latency, uncertain outcomes, stale
    completions, recovery duration, and unresolved provenance links.
  - [ ] Add failure-injection tests at every durable transition.

- [ ] **7. Verify and document**
  - [ ] Document generation semantics, commit points, uncertainty, privacy,
    retention, update integration, and the limits of reproducibility.
  - [ ] Run `cargo fmt --all -- --check`.
  - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
  - [ ] Run `cargo test --workspace --all-features`.
  - [ ] Run `cargo doc --workspace --all-features --no-deps`.
  - [ ] Run real-PostgreSQL, NATS, volume, update-hook, and real-libkrun
    provenance scenarios.
  - [ ] Run `mix precommit` in `web/`.
  - [ ] Run the relevant Playwright inspection scenario.
  - [ ] Run `git diff --check`.

## Completion evidence

Record schema versions, golden generation/run/snapshot IDs, generation CAS and
stale-lease evidence, failure-injection results, retained provenance after
tombstones, UI inspection results, test counts, and deliberate follow-up
tasks.
