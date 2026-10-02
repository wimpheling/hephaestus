# Daemon composition and lifecycle

`hephaestus-app` is the internal production composition root.
`hephaestusd` is its runnable daemon; it is not a public SDK.

## Context facades and composition leaves

The five context facades are the supported contract seams: [`heph-secret`](../crates/heph-core/auth/secret/), [`heph-runtime`](../crates/heph-core/runtime/), [`heph-run`](../crates/heph-core/runtime/run/), [`heph-forge`](../crates/heph-core/forge/), and [`heph-build`](../crates/heph-core/forge/build/). The app directly uses the first four for provider-neutral types; the OCI build adapters use `heph-build` while the app composes their concrete workers. The direct workspace dependencies in [`crates/heph-app/Cargo.toml`](../crates/heph-app/Cargo.toml) below select concrete providers from `heph-std` and cross-context adapters that the facades intentionally do not expose.

Trusted fixture and local-smoke bootstrap binaries live under
[`crates/heph-app/bootstrap/`](../crates/heph-app/bootstrap/). They are part of
the app distribution and use the same provider contracts as the daemon.

| Direct leaves | Composition reason |
| --- | --- |
| `vm-fake`, `vm-libkrun`, `volume-local`, `volume-postgres`, `workspace-local`, `workspace-postgres` | The app selects VM and local filesystem providers and constructs their PostgreSQL metadata adapters; `heph-runtime` exposes only provider-neutral contracts. |
| `run-orchestrator`, `run-postgres`, `run-nats`, `run-runtime-local` | The app installs NATS command handlers/topology, the PostgreSQL run repository, and local run materialization/configuration; `heph-run` intentionally omits those concrete adapters. |
| `secret-broker`, `secret-runtime`, `secret-postgres`, `secret-key-local`, `secret-application`, `secret-domain`, `secret-store` | The app wires the private broker transport, filesystem mount provider, PostgreSQL services, local encryption key provider, secret command/domain types, and encrypted key store; `heph-secret` exposes only the mount lifecycle contracts and manager. |
| `forge-service`, `forge-storage`, `forge-nats`, `forge-postgres`, `git-http`, `git-capability-domain`, `pat-domain`, `pat-postgres` | `forge-service` supplies provider-neutral receive and outbox contracts; standard adapters provide bare-Git storage, PostgreSQL metadata and `gix` receive inspection, NATS publication, smart HTTP, Git capability rules, and PAT services. `heph-forge` intentionally excludes those concrete adapters and transport representations. |
| `release-service`, `release-artifact-store`, `release-domain`, `release-postgres`, `review-domain`, `review-git`, `review-postgres`, `review-nats`, `review-service` | Release and review are forge subcontexts with their own application contracts, immutable artifact storage, trusted Git publication, NATS publication, PostgreSQL adapters, and durable control services; they are outside the top-level forge facade. |
| `registry-domain`, `registry-http`, `registry-notification`, `registry-notification-http`, `registry-postgres`, `registry-publisher`, `registry-reconciler`, `registry-token`, `registry-zot` | Registry contracts, notification and token transports, PostgreSQL state, reconciliation, and Zot/publisher integrations are a separate forge subcontext and require direct composition. |
| `build-orchestrator`, `build-postgres`, `oci-builder-postgres`, `oci-builder-runtime-local`, `oci-builder-worker` | `heph-build` supplies build DTOs and ports; the app must select the concrete build executor, PostgreSQL job stores, local OCI runtime, and worker implementations. |
| `builder-catalog-application`, `builder-catalog-domain`, `builder-catalog-postgres` | These are image-catalog APIs used by the catalog RPC and root-image setup, outside the build facade. |
| `runtime-types` | Shared platform identifiers such as `RunId` and `CommandId` are intentionally not re-exported by a context facade. |

Other direct packages in the manifest belong to independent platform, authorization, control-plane, event, gateway, identity, mailbox, RPC, or runtime-authority contexts and therefore are not replaceable by these five facades. `forge-domain`, `run-domain`, `vm-trait`, `workspace-domain`, and `volume-trait` have no remaining app dependency edge; their app callers use the corresponding facade.

The independent direct packages are `agent-config`, `rpc-proto`, `capability-domain`, `authz-domain`, `authz-postgres`, `runtime-authority`, `runtime-authority-postgres`, `runtime-handoff-local`, `runtime-git-authority`, `runtime-git-authority-postgres`, `control-plane-postgres`, `event-application`, `event-postgres`, `gateway-domain`, `gateway-edge`, `gateway-postgres`, `identity-domain`, `identity-application`, `identity-oidc`, `identity-postgres`, `mailbox-domain`, `mailbox-dispatch`, `mailbox-postgres`, and the dev-only `brokered-egress-domain`. They provide their own platform schema, generated transport, authorization, persistence, event, gateway, identity, mailbox, or runtime-authority boundaries.

## Lifecycle

`HephaestusApp::build` validates static configuration, checks that PostgreSQL
has exactly the application binary's expected migration version applied and the
Mélange dispatcher, resolves storage and VM dependencies, and connects PostgreSQL
and NATS. It does not bind listeners or
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

`crates/heph-app/tests/golden.rs` seeds only the initial identity and
repository metadata before startup. After the readiness barrier, it interacts
through real Git smart HTTP with a signed bearer token, runs an agent that
changes the writable workspace, and waits for the persisted
`result.completed` event. It verifies the controlled result ref, exact input
parent, and imported tree. No receive, outbox, NATS, or orchestrator shortcut
is exposed by the test harness.

The browser and local-smoke seed command
(`crates/heph-app/bootstrap/src/bin/hephaestus-e2e-seed.rs`) is a deliberately
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
