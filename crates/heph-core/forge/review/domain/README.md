# Purpose

`review-domain` gives human review and run-control requests a durable command
shape. It identifies the authenticated actor, repository perimeter, target run
or proposal, and requested control operation so a browser action can be
validated and delivered through the control plane.

# Responsibilities

`ControlCommand` supports cancel and retry for runs and approve or reject for
review proposals. Its `validate` method requires each operation to carry the
matching target kind, preventing a command from accidentally applying a review
action to a run or vice versa. Opaque request and proposal IDs provide stable
idempotency and audit correlation.

The command carries the actor and repository so application services can apply
current authorization before creating an effect. `CONTROL_EXECUTE_SUBJECT`
provides the durable event subject used after the committed control request is
ready for delivery.

# When

Use this crate when a browser or control-plane adapter turns an authorized
request into a durable command. Construct `ControlCommand`, call `validate`,
and publish the committed command for `review-service` to execute.
