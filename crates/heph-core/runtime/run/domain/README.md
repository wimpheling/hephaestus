# Purpose

`run-domain` gives the runtime one durable state machine for an exact agent
execution. It records the selected instance revision and release, repository
attachment, command identity, optional state volume, VM binding, outcome, and
failure details so retries and recovery can reason about the same run.

# Responsibilities

`StartRun` and `CancelRun` are idempotent command values. `RunState` constrains
movement from queued work through leasing, provisioning, starting, running,
outcome, and cleanup; `RunOutcome` remains available after transient resources
have been released. The model keeps cancellation and invalid transitions
explicit, so cleanup must appear as a valid lifecycle transition and success
must have a recorded execution outcome. The orchestrator and its providers still
perform the cleanup effects. It contains identifiers and decisions, while
persistence and provider effects stay behind the run orchestrator ports.

# When

Use this crate when creating or validating a durable run command or applying a
state transition in a repository adapter:

```rust
if run.state.can_transition_to(RunState::Provisioning) {
    // persist the transition through the run repository
}
```

Store the exact release, revision, attachment, and command identifiers with
the run before work reaches a VM.

`FreshRunExecutionProfile` is trusted constructor configuration for new
producers: `LegacyScalar` or `OwnedCanonical`. It has no historical option and
conveys no source, delegation, provider ownership or physical cleanup authority.
