# Verify browser reconnect and restart semantics

The control-plane refactor has unit and subsystem coverage for cursor resume,
duplicate suppression, and restart recovery, but still lacks one isolated
end-to-end proof through the running Phoenix browser session.

## Scope

- Start the complete development environment from a disposable local state
  root and seed one deterministic organization/project journey.
- Open a page with a live product-event stream and record its committed cursor
  and rendered event identifiers.
- Restart the daemon and the Phoenix/RPC channel without changing the durable
  PostgreSQL/NATS state.
- Reconnect the browser stream from the recorded cursor and verify that events
  are resumed exactly once, with no duplicate visible transition and no
  duplicate mutation side effect.
- Repeat once across a supervisor restart so process ordering is covered.

## Acceptance criteria

- The browser reconnects without a full-page refresh.
- The resumed cursor is the last committed cursor before restart.
- Each post-cursor event is rendered once, in order.
- Mutation receipts and external side effects have one durable occurrence.
- Access revocation and retention-gap responses remain fail-closed.

## Notes

This is intentionally separate from the clean-state command audit. The audit
can be completed with `cargo dev state clean/reinit` in an isolated root; this
ticket requires a live browser and controlled process restart timing.
