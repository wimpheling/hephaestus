# Live review control plane

Phase 5 adds a Phoenix LiveView product surface over the durable identity,
authorization, run, workspace, artifact, and result records produced by the
Rust core.

## Trust boundaries

Browser login uses an OIDC authorization-code flow with state, nonce, issuer,
audience, expiry, and signature validation. The verified issuer and subject
must map to one active internal user. Flexible verified claims are persisted
to `user_profiles`.

Phoenix has no database, NATS, repository, artifact-store, filesystem, or
subprocess authority. After browser OIDC establishes the session, every
protected query and command crosses the authenticated RPC mediator over a
generated, typed client. The Rust application derives the actor from trusted
session context, applies authorization, and keeps PostgreSQL transactions and
RLS defense in depth behind its application boundary.

LiveView updates come from typed, product-scoped RPC streams. Phoenix resumes
from committed cursors and treats terminal authorization failures as typed
denials; the Rust stream handler re-authorizes subscriptions. Event payloads
are bounded presentation data, not database notification hints or
capabilities.

## Durable controls

The browser inserts an immutable `control_requests` intent. A database trigger
writes `hephaestus.control.execute` to the transactional outbox. The Rust
consumer loads the authoritative request, validates the delivery, restores
actor context, calls the generated Mélange authorization function, records a
structured authorization audit, and performs the command.

- Cancel emits a durable orchestrator cancellation command.
- Retry creates a new run request for the exact repository, commit, ref,
  configuration hash, receive, and agent, with a new attempt number.
- Reject closes the proposal without changing Git.
- Approve validates result provenance and performs
  `git update-ref <target> <result> <input>`.

Approval is a compare-and-swap fast-forward, not a merge. The result commit
already has the exact input commit as its parent. If the target no longer
equals that input, the proposal becomes `conflicted`; the system never rebases
or merges automatically.

## Browser golden path

[`scripts/run-ui-e2e.sh`](../scripts/run-ui-e2e.sh) creates fresh PostgreSQL
and JetStream services, applies normal migrations, seeds an OIDC identity and
repository, starts the Rust daemon with a deterministic VM fixture, starts an
isolated Phoenix process with only browser/OIDC and RPC configuration, and runs
Playwright in Chromium. The gate asserts that Phoenix has no Git executable,
database/NATS/storage environment variables, or product-storage mounts.

The journey proves:

1. browser OIDC login;
2. a smart-HTTP push appears on an already-open repository page without reload;
3. the exact run, persisted lifecycle/log events, metrics, patch, artifact
   manifest, and controlled result proposal are visible;
4. approval atomically advances the target ref and is durable in PostgreSQL;
5. a second push and result can be rejected without changing its target ref.
