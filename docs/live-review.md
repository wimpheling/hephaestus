# Live review control plane

Phase 5 adds a Phoenix LiveView product surface over the durable identity,
authorization, run, workspace, artifact, and result records produced by the
Rust core.

## Trust boundaries

Browser login uses an OIDC authorization-code flow with state, nonce, issuer,
audience, expiry, and signature validation. The verified issuer and subject
must map to one active internal user. Flexible verified claims are persisted
to `user_profiles`.

Every protected web operation uses one PostgreSQL transaction with
transaction-local actor and request IDs. The web login role is a non-owning,
non-`BYPASSRLS` member of the application role. Reads are filtered by RLS;
commands are also checked by database policies and re-authorized in Rust before
an external side effect.

PostgreSQL `NOTIFY` payloads are hints, not data or capabilities. They contain
only a run identifier. Each run LiveView authorizes before subscribing,
periodically re-authorizes, and re-fetches through RLS after every wake-up.
Repository wake-ups similarly trigger an RLS-filtered re-fetch.

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
repository, starts the Rust daemon with a deterministic VM fixture, starts
Phoenix under a normal RLS-constrained role, and runs Playwright in Chromium.

The journey proves:

1. browser OIDC login;
2. a smart-HTTP push appears on an already-open repository page without reload;
3. the exact run, persisted lifecycle/log events, metrics, patch, artifact
   manifest, and controlled result proposal are visible;
4. approval atomically advances the target ref and is durable in PostgreSQL;
5. a second push and result can be rejected without changing its target ref.
