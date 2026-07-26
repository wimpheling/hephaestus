# VM testing

Hephaestus separates provider-neutral behavior from backend mechanisms so each
new VM provider can prove the same public contract without inheriting libkrun,
KVM, `passt`, Unix-socket, or cgroup assumptions.

## Provider conformance

The `vm-conformance` crate contains tests expressed only in terms of
`vm-trait`. A provider supplies a `ProviderHarness` that returns:

- a provider instance scoped to the test suite;
- a valid long-running `VmSpec` for a requested identifier;
- optional observable capabilities such as readiness events;
- a provider-local assertion that resources were released.

Provider crates invoke `provider_conformance_tests!` to generate independently
named fast tests, or call `lifecycle_suite` when setup is expensive and a
single prepared environment should be shared. The suite covers provisioning
without implicit start, concurrent and repeated start/stop/destroy behavior,
concurrent and cached waits, terminal event uniqueness, identifier collision
and reuse, cleanup, and provider-neutral invalid specifications.

A future cloud provider should implement the same harness using its own
ephemeral project, account, or emulator fixture. Backend capability tests
remain in that provider's crate; the shared suite must never inspect a worker
process, local path, cgroup, TAP device, `passt` process, or libkrun call.

## Fast backend tests

`vm-libkrun` runs its hardware-independent tests on every change. They use:

- a recording `KrunApi` implementation for exact FFI conversion, typed error,
  failure-injection, and context-release checks;
- a `WorkerSpawner` seam for startup failure and cleanup checks;
- mock workers for lifecycle concurrency, event ordering, subscriber lag,
  cached exit, and paused-clock timeout checks;
- framed-protocol round trips and malformed, truncated, oversized, duplicate,
  and out-of-order message checks;
- filesystem fixtures for allowlisted roots, symlink and traversal escapes,
  missing paths, wrong file types, resource limits, and raw-format retention;
- pure `passt` argument and port-reservation tests.

Run all fast tests with:

```sh
cargo test --workspace --all-features
```

## Fedora/KVM integration

The gated integration suite runs as a non-root account on a prepared Fedora
host. It performs real libkrun boots and verifies guest readiness, stdout,
stderr, metrics, persistent raw-disk I/O, read-only repository and writable
workspace mounts, disabled networking, DNS/TCP/UDP egress, loopback-only
ephemeral ingress, graceful cancellation, forced cleanup, readiness timeout,
generic provider conformance, and absence of leaked runtime directories,
cgroups, workers, or `passt` processes.

The reproducible runner creates all mutable fixtures, uses a digest-pinned
Fedora root filesystem, refuses UID 0, and checks that host interfaces and
routes are unchanged:

```sh
scripts/run-libkrun-integration.sh
```

Direct test invocation is gated by `HEPHAESTUS_LIBKRUN_INTEGRATION=1`; see the
[libkrun backend documentation](vm-libkrun.md) for the required environment.

## Scheduled testing

High-volume concurrency, interruption campaigns, long-running output
backpressure, fuzz-corpus execution, and resource-growth measurements belong
on the prepared Fedora runner as nightly and release jobs. Regression cases
found there should be reduced into the fast provider-neutral or backend suite
whenever they no longer require real hardware.
