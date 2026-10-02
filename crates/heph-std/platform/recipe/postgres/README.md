# Recipe deployment admission ledger

This declared PostgreSQL adapter provides three inherent operations:
`admit_install`, `inspect`, and `admit_remove`. It performs no provider work and
does not implement the effect methods of `DeploymentRepository` yet.

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
forced row security. Provider claims, ambiguous-result reconciliation, completion,
and their further schema guards belong to the next slice.
