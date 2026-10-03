# Purpose

`release-service` applies release and instance commands to the durable
project workflow. It turns a completed build into a release, imports that
release into a project, creates exact repository attachments, revises
capabilities and parameters, and coordinates update hooks and recovery.

# Responsibilities

The command DTOs carry stable `ReleaseCommandKey` identities and expected
revision IDs, so retries and compare-and-swap checks preserve immutable
history. Capability revision commands validate the complete binding set and
return runnable status with safe diagnostics; update commands keep candidate
release family, broker rule copies, hook result, and operator recovery choice
explicit.

The service also owns UI installation and serving contracts. It creates short
browser handoffs, authenticates child sessions against the expected generation,
projects requests to active UI artifacts or gateway routes, and emits request
audit events with anonymous, actor, or verified context as appropriate.

# When

Use the command types when a worker or authenticated application request needs
to import a release, revise an instance, attach a repository, run an update
hook, or install UI. Pass each command to the corresponding application port
with its expected revision and command key.

`ImportAgentWithVolumes` pins the release/export, predicted instance/revision and
grant IDs, ordinary parameters, policies, and exact named resource selections.
The adapter validates complete bindings and hashes normalized inputs before
activation. Named dispatch remains independently closed until the runtime
supports the profile; immutable revision completeness is preserved.

`RequestInstanceRemoval` carries an expected instance version and predicted
permanent admission identity. It admits cancellation under current management
without claiming VM cleanup, lease release, volume deletion, or a terminal
instance tombstone. `InstanceVolumeStatus` reports that live boundary without
exposing provider handles or encryption material.

Use `InstanceExecutionService` to activate an installed recipe instance once its
resources are ready, then request runs of its saved revision. Activation checks
current management and source-use permissions; Invocation checks execution and
source-use permissions. Exact instance, revision, and command identities preserve
the original receipt while replay rechecks current permissions. Activation replay
keeps a subsequently closed gate closed. These interfaces define admission;
adapters provide the transaction commits and scheduling behind them.

`InstanceExecutionConfiguration` is checked server composition data shared by
qualified adapters. Its v1 canonical JSON preserves the existing admission
stamp. Construction, typed getters and encoding perform no IO and grant no
caller authority or physical ownership/readiness proof; composition must validate
the actual provider and volume roots separately.
