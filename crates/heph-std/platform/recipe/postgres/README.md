# Recipe deployment ledger

This declared PostgreSQL adapter implements `DeploymentRepository`: admission,
inspection, fenced effect claims, verified completion, ambiguity, reconciliation,
and terminal command receipts. It performs no provider work.

Installation resolves canonical declarations again inside the actor transaction
using exact authoritative release exports, typed parameters, secret schemas,
effective volume slots, capability requirements, and safe external volume facts.
Caller-built validated intent is compared with this resolution and never trusted
as catalog evidence. Installation, including resumed admission, checks live
project, source, and external permissions before looking up receipts.

The ledger stores immutable admission evidence for historical hydration. This
evidence reconstructs fingerprints after a release is withdrawn or an external
resource disappears. It is not reusable authority. Cleanup requires project
management and exact existing owned-resource management, without source access
or consumer authority over retained external references. No consumer grants are
created by admission.

Recipe identity/version is unique within a project. Deployment keys and deleted
deployment tombstones are never reused. Actor, operation, and stable idempotency
identity bind commands; request IDs are append-only attempt provenance. Receipts
store the exact committed project event metadata. Audit, admission state,
receipts, and committed outbox events share the same transaction. Denials use the
existing independently committed audit pattern.

Migration 0104 guards immutable identity, intent, evidence, command receipts,
provenance, bounded progress vocabularies, and legal lifecycle transitions under
forced row security. Migration 0106 adds append-only execution attempts, worker
verification receipts, exact transition proofs, and separate immutable terminal
results. Claims increment deployment/resource CAS and execution fencing generation
before a provider can run. Ambiguous and failed actions keep their active fence;
only authoritative reconciliation can release it. Expiry proves no absence or
attachment safety. Cleanup reverses dependencies, enforces immutable retain/delete
policies, and never mutates external references.

Public `EffectClaim` and `EffectEvidence` values are untrusted comparisons against
stored facts. Completion and proved reconciliation require a receipt recorded
through `PostgresEffectVerificationRecorder` with the trusted worker database role.
That recorder is a backend composition boundary and verifies ledger identity; the
owning provider must first establish action-specific facts. In particular, a create
needs durable operation-bound ownership provenance. A preexisting independent
volume with matching predicted ID, capacity, and filesystem UUID cannot be adopted
or deleted. Definitive absence must exclude such an unowned resource. Provider
ownership seals and actual provider invocation remain future integration work.

Fresh effects and terminal/replayed commands require current authenticated actor
authority. Trusted worker observations can be appended without synthesizing an
identity from a historical actor. Removal excludes source access; observation of
an original owned install fence after removal was admitted may resolve cleanup
safety under a separately admitted current manager command, without borrowing the
original actor identity or reopening installation. The immutable target attempt
and the current reconciliation command/provenance remain separately recorded. Terminal completion is distinct from its stable
original admission receipt.

The ignored PostgreSQL tests use explicit trusted worker proof fixtures to verify
ledger semantics, forgery rejection, CAS, replay, cleanup, SQL guards, and a frozen
0105-to-0106 database upgrade. They do not prove provider, filesystem, or VM safety.
