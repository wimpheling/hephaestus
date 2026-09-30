# Purpose

`review-service` executes durable human controls and the controlled Git effect
that approves a run result. It lets users cancel or retry runs, reject
proposals, or fast-forward a target ref only after the result still matches
the proposal's original input.

# Responsibilities

`ReviewControlService::execute` validates the command and sends cancel, retry,
and rejection through one repository transaction. Approval first asks the
repository to authorize and claim the proposal, then asks `ReviewGit` to
publish with compare-and-swap, and finally records the outcome. A moved target
becomes `Conflicted`, while an already completed command can safely replay.

The service exposes outbox ports so a committed control request is published
and retried durably. Repository location and Git publication use explicit
ports, and finalization records the external Git effect in its own subsequent
durable step.

# When

Use this service for a command delivered from the committed control outbox.
Construct it with `ReviewControlService::with_git`, then call `execute`; the
returned `ControlOutcome` tells the UI whether the action completed, was
denied, was already complete, or conflicted.
