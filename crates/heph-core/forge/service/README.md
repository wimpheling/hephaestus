# Purpose

`forge-service` coordinates repository metadata and Git receive processing.
It turns an accepted ref update into durable run and isolated-build requests,
returns receive outcomes for the caller, and publishes follow-up work through
a transactional outbox.

# Responsibilities

`CreateRepository` carries the project, default branch, visibility, and run
trigger policy needed to create repository metadata and storage. Receive
processing returns the exact receive ID, commit and ref-bound run requests,
build requests, and configuration diagnostics in `ReceiveResult`.

`ForgeOutboxStore` exposes pending records and publication acknowledgements so
NATS or another event adapter can retry without losing a committed receive.
Typed errors distinguish authorization, metadata, Git inspection, storage, and
serialization failures, allowing callers to retry the right class of work.

# When

Use this crate at repository creation and after an authorized Git transport
has accepted an exact receive. Build a `CreateRepository` for creation, or
pass the receive data to the Forge application service and publish the
resulting outbox records only after its transaction commits.
