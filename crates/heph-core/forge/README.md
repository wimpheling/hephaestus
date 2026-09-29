# Forge core

Forge owns the project and repository lifecycle and the durable path from an
exact source commit to a reviewed, immutable release. Its logical scope also
includes isolated builds, review controls, and OCI registry publication.

## Contracts and decisions

- [`domain/`](domain/) defines project, repository, Git receive, commit, ref,
  and run-request values.
- [`service/`](service/) defines provider-neutral receive processing and the
  transactional outbox boundary. A receive is idempotent for its repository,
  commit, ref, configuration hash, and receive identity.
- [`git-capability/`](git-capability/) defines bounded Git operations, ref and
  path matching, transfer limits, and mutation policy.
- [`git-http/`](../../heph-std/forge/git-http/) is the authorized streaming
  smart-HTTP transport;
  it keeps native Git invocation behind a bounded, cleared environment.
- [`storage/`](../../heph-std/forge/storage/) owns canonical bare-Git
  filesystem and process storage.
- [`postgres/`](../../heph-std/forge/postgres/) owns PostgreSQL repository
  metadata and receive persistence, including exact-commit `gix` inspection.
- [`nats/`](../../heph-std/forge/nats/) publishes committed forge outbox
  records through JetStream.
- [`build/`](build/) and its `build-*` children define exact-source isolated
  build jobs, durable job ports, and immutable artifact import.
- [`review/`](review/) owns durable review controls and controlled result
  approval.
- [`review-git`](../../heph-std/forge/review/git/) is the trusted Git adapter
  that publishes an approved result ref.
- [`release/`](release/) owns immutable releases, project instances, revisions,
  attachments, updates, and artifact storage.
- [`registry/`](registry/) owns registry publication records, evidence,
  notification contracts, reconciliation, and token scope.
- [`pat/`](pat/) and [`pat-postgres/`](../../heph-std/forge/pat-postgres/) own
  developer token contracts and their hash-only persistence.

Build and release decisions are explicit. A normal run consumes an immutable
published release; it does not build implicitly. Source, configuration hash,
artifact manifest, release identity, revision, and policy provenance remain
available for inspection. Registry publication is controlled by Forge-owned
records and evidence rather than by an arbitrary guest.

## Boundaries and implementations

Forge core defines ports and durable meaning, but it does not own PostgreSQL,
NATS, bare-Git storage, `gix`, a host process, or a registry daemon. Those
concrete adapters are in [`heph-std/forge/`](../../heph-std/forge/) and use the
root migrations where needed. The composition and worker lifecycle are owned
by [`heph-app`](../../heph-app/).

See [`docs/git-forge.md`](../../../docs/git-forge.md),
[`docs/repository-image-builds.md`](../../../docs/repository-image-builds.md),
[`docs/live-review.md`](../../../docs/live-review.md), and
[`docs/releases-and-instances.md`](../../../docs/releases-and-instances.md).
