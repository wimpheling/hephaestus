# Workspace providers

The workspace providers implement exact source materialization and controlled
result publication. [`postgres/`](postgres/) persists request, workspace,
runtime-Git, result, artifact, and lifecycle metadata. [`local/`](local/)
validates administrator-owned roots, materializes exact Git input, manages
runtime-Git worktrees, seals outputs, and publishes approved refs.

The run orchestrator asks the local manager to prepare a workspace after the
release and authority snapshot are fixed. The manager records preparing and
active state through PostgreSQL, exposes read-only source input or a
capability-scoped runtime-Git bridge, and seals a separate result area. At
completion it imports declared files and performs a controlled compare-and-swap
publication. Recovery removes only owned staging trees and reconciles metadata
for interrupted runs.
