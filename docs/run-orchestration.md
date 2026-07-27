# Durable run orchestration

Phase 1B is single-host while keeping VM and volume interfaces
provider-neutral. PostgreSQL is authoritative for agents, local-volume
ownership, exclusive leases, runs, events, the command inbox, and the
transactional outbox. Raw ext4 backing files live beneath a configured
persistent volume root and never beneath libkrun's transient runtime root.

## Crates

- `runtime-types`: shared stable identifiers.
- `volume-trait`: provider-neutral volume and fenced-lease contract.
- `volume-local`: PostgreSQL-coordinated local raw-file implementation.
- `run-domain`: durable states, outcomes, and commands.
- `run-orchestrator`: PostgreSQL repository, VM coordination, restart
  reconciliation, and JetStream outbox publication.

## Lease recovery

Lease expiry is only evidence that supervision was interrupted. It does not
authorize another writable attachment. Recovery changes the volume to
`Recovering`, calls `VmProvider::cleanup_orphan` for the previous run's VM ID,
and releases the lease only after the provider confirms that all processes are
reaped and disks detached. A cleanup error retains the lease.

At supervisor startup, call `RunOrchestrator::recover_after_restart`. In
addition to expired leases, it closes the crash window where VM destruction
and lease release completed but the final `CleanedUp` transition did not.
Queued and pre-lease runs are left for their durable command to redeliver.

Every local volume records `host_id`. A host refuses to attach a volume owned
by a different host. This is an implementation constraint of Phase 1B, not a
property of `volume-trait`.

## JetStream

Commands:

- `heph.run.command.start.v1`
- `hephaestus.run.start` (forge-originated start command)
- `heph.run.command.cancel.v1`

Lifecycle distribution:

- `heph.run.event.lifecycle.v1`

Recommended streams are `HEPH_RUN_COMMANDS` with a durable pull consumer named
`run-orchestrator-v1`, and `HEPH_RUN_EVENTS` with limits-based retention.
Incoming `command_id` values are retained in the PostgreSQL inbox for
unbounded duplicate handling. Outbox IDs are sent as `Nats-Msg-Id` to suppress
publication retries within JetStream's duplicate window. Long-running start
handlers send progress acknowledgements while the VM runs; a supervisor crash
stops those acknowledgements and makes the command eligible for redelivery.

## Opt-in integration tests

The PostgreSQL repository and local-volume tests run only when
`HEPHAESTUS_POSTGRES_TEST_URL` is set. The JetStream delivery test additionally
requires `HEPHAESTUS_NATS_TEST_URL`. These tests are skipped in an ordinary
workspace test run.

The full two-VM persistence scenario is also excluded from ordinary CI because
it requires KVM, libkrun, delegated cgroups, an ext4-capable guest fixture, and
PostgreSQL. The integration runner builds and cleans up the guest fixture:

```sh
HEPHAESTUS_PHASE1B_INTEGRATION=1 \
HEPHAESTUS_POSTGRES_TEST_URL=postgres://... \
./scripts/run-libkrun-integration.sh
```

The test can alternatively be run directly by providing the
`HEPHAESTUS_LIBKRUN_*` paths described in `vm-libkrun.md`, plus
`HEPHAESTUS_LIBKRUN_WORKER` pointing to the host worker binary.

The fixture's guest image must reserve numeric UID and GID `10001` for
`heph-agent`. Production images and writable shared directories must use the
same numeric ownership until the runtime supports ID-mapped mounts.
