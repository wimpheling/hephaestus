# Purpose

`build-postgres` persists the state machine used by `build-orchestrator`.
It reads build requests and configuration from PostgreSQL, claims execution
and verification work, stores retry and recovery state, and records the
artifact manifest and release transition produced by a successful build.

# Responsibilities

`BuildRepository` is implemented by the PostgreSQL adapter through the
verification, lifecycle, and model modules. Claims are leased and state
transitions are checked before updates, so a restarted worker can recover an
unfinished VM or finalization step without losing the original build identity.
The adapter validates stored configuration, exact commit and ref values,
artifact projections, authorization context, and release links before returning
them to the orchestrator.

The implementation keeps SQLx row conversion and database writes inside the
PostgreSQL boundary while returning the provider-neutral repository errors and
`ClaimedBuild` values expected by the worker.

# When

Inject `BuildRepository` from this crate into `BuildExecutor` and its verifier.
The executor calls `claim`, marks each state transition, and uses the
verification methods to complete or reset work after a build attempt.
