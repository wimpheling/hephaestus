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

Migration 0109 adds `retire_external_observation` for orphaned external read-only
verification during removal. Exact current cleanup authority, target claim,
generation, and deployment/resource CAS are required. It appends a distinct
`observation_retired` journal/event, clears only the active observation, preserves
unverified installation history and diagnostics, and leaves the reference retained.
No worker receipt, external lookup/access, or provider fact is used. Unchanged
replay retains original operation provenance under fresh live request authorization;
new operations require fresh current actor/request provenance. Retirement tests
assert no successful provider receipts and no owned creation fence cancellation.

`PostgresExecutionPreparationRepository` selects the platform/profile through its
backend constructor. It hydrates historical intent, validates the sole original
Install command, rechecks live project/source/external authority, and freezes the
complete typed import graph before any claim, mapping, registration or import.
Replay reconstructs every stable field and rejects configured policy/version or
source-observation drift without overwriting original bytes or receipt. Historical
inspection requires project management and does not require source access.

The adapter requires the private 0113 preparation proposal, which remains
unpublished pending native verification. Its protected actor writer creates the
fresh event/audits and every header/child together; actors cannot insert rows
through raw table DML and workers cannot fabricate preparation. Linked audit IDs
come only from that writer's fresh INSERT results. Native fixtures exercise
metadata, roles, rollback and genuine 112-to-113 preservation; they do not prove
physical backing, actual typed import execution, provider outcomes or app startup.

## Retaining a ready owned volume

A current project and volume manager can close attachments, let the known run
owners drain their workloads, verify the original disk without writes, and retain
its data. Removing an installation does not borrow the original installer's
identity or require continued access to its old Source.

The service keeps data held when a consumer, lease, owner, original creation, or
physical disk cannot be checked. Actual journal custody stays owned through the
worker fact and manager's commit, including caller cancellation. Retention leaves
attachments closed; a new consumer needs fresh authorization and an explicit
reopening receipt. The current reopening path requires an originally completed
installation. An unfinished but physically ready birth retains its distinct
positive fact and remains held for reuse.

This opt-in backend path does not initialize roots, format disks, rewrite original
installation receipts, reset lease generations, enable named dispatch, or provide
physical VM shutdown evidence from database status. The checked scope pins come
from the configured provider and remain separate from volume-root ownership.

### Fresh manager continuation

The configured repository admits a new current manager while preserving the
original removal claims and permanent instance request. It reloads the complete
saved preparation, checks every materialized owned cleanup permission, reserves
the original command and instance identities before locking parents, and commits
the new command, event, audits and immutable continuation record together.
Continued source access and the previous manager's credentials are unnecessary.

`with_remove_resume` is an explicit server-composition choice. The default
constructor returns Unsupported for this admission and keeps existing APIs and
public-schema behavior. Scope labels must come from the actual checked provider;
they are trusted comparison data, independent of disk ownership. Current platform
or scope drift denies replay. Historical or foreign runtime scope stays held.

Selected original owned Drain, Detach, Delete and Retain claims can be reconciled
under the new command. Original claim, request, key, version and creator are never
rewritten. Already committed reconciliation replays compare its immutable
fingerprint before requiring a current active attempt. New observations still
require the exact active generation and genuine worker verification; Unproven
retains the original fence. The configured `terminal_history` reader correlates
instance deletion with its original creation, closure, worker verification and
protected tombstone. These database observations support the owning cleanup
coordinator; actual VM cleanup must complete before it can report removal.
`load_completed_remove` lets a currently authorized manager recover the final
removal receipt after a restart or lost reply. It checks the whole saved
deployment, all owned resource permissions and the original completion history
before returning the receipt. This avoids starting another removal command for
an already removed deployment.

The private successor appends after the frozen preparation, retention and readonly
instance safety/closure prerequisites. It adds one immutable admission table and
an independently qualified static cross-Remove transition branch. The inverse
branch comparison is byte-exact to the frozen prior guard. No startup, provider,
transport, activation or runtime drain implementation is included.

The authored native fixtures exercise database admission, immutable original closure
preservation, Unproven reconciliation/replay and lock ordering when executed. Zero-Run data is
not a physical VM absence proof. Cleared-active positive replay remains for the
owning drain gate with genuine runtime evidence; no fake cleanup receipts are seeded.
