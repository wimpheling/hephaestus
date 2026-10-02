# Purpose

`vm-trait` defines the provider-neutral contract for running an isolated
guest. It lets the run orchestrator request a VM with an explicit filesystem,
network, command, resource, private-service, and runtime-authority setup while
leaving libkrun and other host implementations in their adapter crates.

# Responsibilities

`VmSpec` makes every guest input visible: approved root filesystem and disks,
bounded mounts, network mode and forwards, resource limits, guest command, and
the credentials used by trusted runtime bridges. `VmProvider` and `VmInstance`
cover provisioning, start, event subscription, private HTTP connections, stop,
and orphan cleanup. Errors, exits, metrics, and logs are typed so the host can
persist useful evidence without treating guest output as authority. Providers
must honor the spec and cleanup contract; they do not gain permission to read
secrets or publish repository results merely by running a guest.

Named `VmGuestVolume` entries bind a bounded slot, filesystem UUID, controlled
guest path, and exact access mode to an existing disk ID. Validation rejects
aliases, overlapping paths, platform-mount conflicts, and mixed legacy labels.
These are guest execution contracts; authoritative resource grants, durable
attachment evidence, and run admission belong to the orchestration layer.
Typed mount metadata itself grants no authority.

# When

Use this crate when implementing a VM adapter or assembling a run specification
for the orchestrator:

```rust
let instance = provider.provision(spec).await?;
let events = instance.subscribe_events();
```

Feed the resulting instance through the run lifecycle and call orphan cleanup
for provider resources left by an interrupted process.
