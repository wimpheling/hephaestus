# VM providers

The VM providers implement the core `VmProvider` and `VmInstance` contracts.
[`fake/`](fake/) gives lifecycle and private-service tests deterministic
behavior without processes. [`libkrun/`](libkrun/) runs Linux guests through a
dedicated unprivileged worker with libkrun, libkrunfw, KVM, passt, and cgroup-v2
limits.

Both providers validate the same `VmSpec`, expose bounded events and exits, and
make start, stop, wait, private HTTP, destroy, and orphan cleanup explicit.
The run orchestrator therefore gets the same caller outcome in tests and on a
prepared Fedora host: a ready instance, typed exit and events, or a cleanup
error that remains visible for recovery.

The libkrun implementation keeps provider file descriptors, processes, and
guest setup in the worker process. Canonical image, disk, and mount roots,
runtime authority sockets, service identity, KVM, and cgroup settings are
validated before provisioning so repository input cannot select arbitrary host
paths or escape resource limits.
