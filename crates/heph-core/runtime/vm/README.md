# VM contracts

The VM branch describes the smallest host-to-guest lifecycle that the runtime
needs. [`trait/`](trait/) models a provider that provisions an isolated guest,
boots the approved runtime, attaches bounded disks and mounts, exposes private
services, streams events, and cleans up after exit or interruption.

Run orchestration supplies a `VmSpec` after resolving immutable release data,
source input, authority handoff, and resource policy. A provider turns that
spec into a `VmInstance`; the orchestrator owns the state transition around
provisioning, readiness, cancellation, event persistence, and cleanup.

The contract keeps guest input explicit. Network mode, port forwards, mounts,
resource limits, guest command, and runtime credentials are represented as
typed values so an adapter cannot infer extra access from a repository path or
an arbitrary command. Private service and mailbox paths remain bounded by the
same request and response rules used by the gateway and runtime.
