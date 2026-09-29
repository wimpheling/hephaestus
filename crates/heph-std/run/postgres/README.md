# Purpose

`run-postgres` persists the durable run state machine behind the core run
orchestrator. It records idempotent start commands, exact release and
attachment provenance, volume and VM bindings, transitions, outcomes,
cancellation, recovery candidates, and ordered VM events.

# Responsibilities

The repository locks and matches command identity before creating a run,
validates every state transition, and writes the transition and bounded event
in the same transaction. It captures runtime-Git provenance and loads the
exact release artifacts, parameters, mailbox event, and update context needed
by local materialization. Authorization and actor context are applied at the
database boundary, while storage errors become typed repository errors. This
prevents duplicate NATS deliveries or stale workers from changing another run's
resources or outcome.

# When

Construct the repository from the control-plane pool and inject it into
`RunOrchestrator`:

```rust
let repository = PgRunRepository::new(pool);
```

The orchestrator receives committed `Run` values and ordered event evidence;
recovery callers can enumerate non-cleaned runs for reconciliation.
