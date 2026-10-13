# Purpose

`vm-fake` is a deterministic in-memory implementation of the VM provider used
by runtime and integration tests. It validates provider-neutral specifications,
allocates repeatable fake ingress ports, emits lifecycle events, and exercises
private guest HTTP without starting a process.

# Responsibilities

`FakeProvider` tracks provisioned IDs and rejects duplicate instances. Its
instances model provisioned, running, exited, and destroyed states, cache
terminal exits, reserve and release user-mode ingress, and reject operations in
the wrong state. A configurable private responder covers broker and gateway
tests while preserving the request and response types of a real provider. The
fake therefore tests orchestration ordering, cancellation, cleanup, and event
handling rather than bypassing those contracts.

`FakeProvider::new_owned(host_id)` creates an explicit test owner. Clones retain
its scope, while each fresh provider receives a different random namespace and
cannot confirm absence for another owner. This models scoped cleanup ordering
without claiming durable physical host ownership. Ordinary `new()` remains
unowned and rejects ownership lookup and scoped orphan cleanup.

# When

Use this provider in unit and conformance tests that need a VM implementation:

```rust
let provider = FakeProvider::new();
let instance = provider.provision(spec).await?;
```

Drive `start`, `wait`, event subscription, and `destroy` through the same run
workflow used by the production provider.
