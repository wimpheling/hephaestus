# Daemon composition and lifecycle

`hephaestus-app` is the internal production composition root.
`hephaestusd` is its runnable daemon; it is not a public SDK.

## Lifecycle

`HephaestusApp::build` validates static configuration, checks that PostgreSQL
has migration version 8 and the Mélange dispatcher, resolves storage and VM
dependencies, and connects PostgreSQL and NATS. It does not bind listeners or
spawn background tasks.

`HephaestusApp::start` first reconciles abandoned isolated builds, run
resources, secret mounts, and committed update decisions. It then binds the
HTTP and private broker listeners, creates the durable JetStream topology, and
starts supervised HTTP, build-command, run-command, broker, and outbox tasks.
It returns `RunningHephaestus` only after:

- the HTTP socket is bound and its server task has started;
- the outbox publisher loop is running;
- the durable run consumer has opened its message stream;
- the durable isolated-build consumer has opened its message stream;
- the private semantic secret broker is accepting provider-forwarded streams.

Startup timeout or early task failure cancels and reaps every task already
started.

`RunningHephaestus::shutdown` stops HTTP and command admission, requests
cancellation of active durable runs, drains supervised work up to the
configured timeout, drains NATS, and closes the PostgreSQL pool.

## HTTP authentication boundary

Git credentials terminate in Axum middleware:

```text
Authorization: Bearer <JWT>
  → signature/issuer/audience/expiry verification
  → (issuer, subject) identity mapping
  → AuthenticatedIdentity request extension
  → PostgreSQL/Mélange repository authorization
  → GitHttpService
```

The middleware removes the header before continuing. The native backend is
invoked by an absolute configured path after `env_clear()` and receives only
the reviewed CGI allowlist.

## VM configuration

The run orchestrator uses the exact validated configuration revision bound to
the durable run request. The application translates its root image, guest
command, resources, state intent, and network profile into one provider-neutral
`VmSpec`. Selecting `FakeProvider` or `LibkrunProvider` changes the backend, not
the committed `agent.toml`.

Immediately before constructing that `VmSpec`, the application checks the
revision's immutable CPU, memory, and network selection against the current
operator policy. `HEPHAESTUS_RUNTIME_POLICY_VERSION`,
`HEPHAESTUS_RUNTIME_MAX_VCPUS`, `HEPHAESTUS_RUNTIME_MAX_MEMORY_MIB`,
`HEPHAESTUS_RUNTIME_ALLOW_BROKER_ONLY`, and
`HEPHAESTUS_RUNTIME_ALLOW_EGRESS` configure that ceiling. A tightened policy
denies a launch that is no longer allowed; it never silently substitutes a
smaller VM or a different network mode. The current policy version is added to
VM metadata for operational correlation.

When workspace mounting is requested, the trusted workspace manager appends an
immutable exact-commit source mount at `/workspace/repo` and a separate writable
copy at `/workspace/work`. The canonical bare repository is never mounted in
the guest. The mount paths in `agent.toml` are requests constrained by this
fixed host policy.

## Golden test

`crates/hephaestus-app/tests/golden.rs` seeds only the initial identity and
repository metadata before startup. After the readiness barrier, it interacts
through real Git smart HTTP with a signed bearer token, runs an agent that
changes the writable workspace, and waits for the persisted
`result.completed` event. It verifies the controlled result ref, exact input
parent, and imported tree. No receive, outbox, NATS, or orchestrator shortcut
is exposed by the test harness.

The browser and local-smoke seed command
(`crates/bootstrap-postgres/src/bin/hephaestus-e2e-seed.rs`) is a deliberately
trusted bootstrap boundary, not a second application API. It creates the
project and repository through the forge's `*_trusted` operations, then seeds
only deterministic fixture relations and release rows that have no public
interactive equivalent. The resulting release contracts use the same
validated runtime, mount, result, and secret-slot shapes consumed by the
production launch path; fixture code must be updated when those contracts
change.

Normal CI injects a hardware-independent result guest that exercises the same
provider-neutral VM contract. Setting
`HEPHAESTUS_APP_LIBKRUN_E2E=1` with the same libkrun host fixture variables
used by the hardware integration suite runs this identical test body,
repository commit, and `agent.toml` through `LibkrunProvider`; only backend
configuration and its host asset paths change.

On a prepared Fedora host, the local runner provisions the pinned guest
fixture, ephemeral PostgreSQL and NATS containers, delegated cgroup, and all
storage roots before running that real-backend variant:

```sh
scripts/run-hephaestus-e2e.sh
```

The runner removes its containers, fixture files, and cgroups on success,
failure, or interruption. Set `HEPHAESTUS_POSTGRES_TEST_URL` or
`HEPHAESTUS_NATS_TEST_URL` to reuse an existing service instead of starting
the corresponding ephemeral container.
