# Purpose

`build-orchestrator` turns a Forge build request into an isolated exact-source
execution and imports the resulting files as release artifacts. It is the
workflow bridge between repository configuration, the VM build contract, and
the release service.

# Responsibilities

`BuildExecutor` claims a build, resolves the exact commit and configuration,
prepares a private workspace, runs the VM through the injected execution
boundary, validates declared outputs, seals the artifact tree, and records a
release draft with stable artifact IDs and a manifest. Its repository port
tracks running, sealed, imported, drafted, failed, and recoverable states.

Verification and restart recovery are explicit: `verify` claims completed
work, checks its artifact and release inputs, and `recover_after_restart`
reconciles builds left behind by a process or VM failure. Results report the
release identity, version, and artifact count for later publication.

# When

Initialize `BuildExecutor` with `BuildExecutorConfig` and a `BuildRepository`,
then call `execute` for a new request or `retry` for a recoverable one. The
executor returns `BuildExecutionResult` after release artifacts are imported.
