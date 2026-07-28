# Daemon composition and lifecycle

`hephaestus-app` is the internal production composition root.
`hephaestusd` is its runnable daemon; it is not a public SDK.

## Lifecycle

`HephaestusApp::build` validates static configuration, checks that PostgreSQL
has migration version 4 and the Mélange dispatcher, resolves storage and VM
dependencies, and connects PostgreSQL and NATS. It does not bind listeners or
spawn background tasks.

`HephaestusApp::start` binds the HTTP listener, creates the durable JetStream
topology, and starts supervised HTTP, outbox, and run-command tasks. It returns
`RunningHephaestus` only after:

- the HTTP socket is bound and its server task has started;
- the outbox publisher loop is running;
- the durable run consumer has opened its message stream.

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
