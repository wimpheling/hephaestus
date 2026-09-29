# Purpose

`capability-audit` makes a runtime capability decision traceable. It records
which immutable authority snapshot and binding were evaluated, what operation
was attempted, and whether the attempt was later used successfully. An
operator can inspect that history for one run without receiving sensitive
request material.

# Responsibilities

After checking a capability, the worker creates a decision event; after the
allowed operation completes, it creates a use event. Both constructors retain
the runtime session, immutable snapshot, binding, operation, request ID, and
authorization model version, making the evidence attributable to one run and
one policy revision.

The repository port lets the worker append these records and lets an
authorized inspector read a run's redacted history. `CapabilityAuditReason`
accepts only bounded machine reasons and `CapabilityAuditPage` limits each
inspection request to 200 events, keeping evidence useful without becoming a
payload or secret transport.

# When

Append a decision immediately after capability evaluation, append a use event
after the operation finishes, and inspect the resulting history when a user
reviews a run:

```rust
let page = CapabilityAuditPage::new(50, None)?;
let events = repository.list_for_run(&identity, run_id, page).await?;
```
