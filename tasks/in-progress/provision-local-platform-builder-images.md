# Provision local platform OCI images explicitly

Owner: Codex

## Outcome

A KVM-capable developer machine can run the Hephaestus stack locally with
Podman-managed infrastructure and, when explicitly requested, build and
install the four standard platform OCI images into the persistent local
Zot/catalog state. Normal `cargo dev` startup never builds, scans,
publishes, or provisions platform images.

## Locked decisions

| Area | Decision |
| --- | --- |
| Stack lifecycle | Keep `cargo dev` as the authoritative local supervisor; do not introduce a parallel Compose file. |
| Infrastructure | PostgreSQL, NATS, Zot, and the OCI build-toolchain remain Podman-managed with named volumes or isolated local state. Heph, Phoenix, OIDC, and libkrun remain host processes. |
| VM support | Accessible KVM is a hard prerequisite; no degraded non-KVM demo mode exists. |
| Image operation | Standard OCI image distribution is an explicit heavy operator action and is never a startup, migration, seed, or CI side effect. |
| Build toolchain | Run Buildah, Skopeo, Syft, Trivy, ORAS, and jq only from a pinned, administrator-built Podman tool image. The host does not need those OCI tools installed. |
| Persistence | Approved local catalog records, release evidence, and OCI layouts survive `cargo dev` restarts. Local installation receipts are removed only by an explicit scoped clean command. |
| Supply chain | Build inputs remain pinned; Buildah construction, Syft SBOM, offline Trivy scan, publication, verification, approval, and catalog provisioning remain separate observable phases. Every approved OCI image is materialized through the same lifecycle and is selectable by any execution contract. |

## Implementation checklist

- [x] Add a `cargo dev platform-images` command group with explicit `status`,
  `build`, `publish`, and `clean` subcommands; reject implicit/default image
  operations.
- [x] Add durable, private local paths for release layouts/evidence and
  installation receipts; include them in scoped inspection and cleanup without
  overlapping existing VM/runtime roots.
- [x] Add a prerequisite check for accessible KVM and rootless Podman only.
  All OCI build tools are supplied by the pinned build-tool image; the local
  command must provision that image rather than require host installations.
- [x] Define and build a pinned OCI build-tool image containing Buildah,
  Skopeo, Syft, Trivy, ORAS, and jq. Verify exact versions/checksums during
  image construction and expose no host socket, workspace, registry, or
  signing-key mount by default.
- [x] Run the explicit platform-image build inside that tool image with only
  the reviewed repository source, dedicated private output, dedicated
  container storage/cache volumes, and narrowly required Buildah isolation
  capabilities mounted.
- [ ] Implement an explicit controlled pinned-base import into local Zot and
  record its upstream digest without enabling general pull-through.
- [x] Implement the explicit four-image build and evidence operation using
  fixed reviewed definitions and a caller-confirmed source revision/timestamp.
- [x] Implement publication, read-back verification, approval, and catalog
  provisioning against local Zot/PostgreSQL. The resulting plain OCI images
  are materialized through the shared image lifecycle and may be selected by
  build or guest execution contracts.
- [x] Make reruns idempotent or fail safely on conflicting immutable release
  evidence; never overwrite an existing reviewed release output.
- [x] Document local installation, expected disk/time cost, status inspection,
  and narrowly scoped cleanup.
- [x] Add focused CLI/state tests and run the relevant smoke plus repository
  quality gate.

## Completion evidence

- [x] Record one local explicit build/install run for all four images and its
  catalog references.
- [ ] Record a `cargo dev` restart that consumes the installed catalog without
  triggering image work.

## Container-build evidence

On 2026-08-05, a rootless Podman run of the pinned
`localhost/hephaestus-platform-build-tools:dev` image completed all four
reviewed layouts for revision `86d0c0f1dfad5cab56b333aad60fdb3c4a9f38af`:
`ubuntu-native`, `rust-ubuntu`, `typescript-node-ubuntu`, and `python-ubuntu`.
The tool container receives no Podman socket or registry/signing credentials.
It receives only the read-only source bind mount, private output bind mount,
named Buildah/cache volumes, and `/dev/fuse`; `BUILDAH_ISOLATION=chroot` keeps
the nested build rootless without a privileged container.

## Local publication evidence

On 2026-08-05, that reviewed release was explicitly published through
`cargo dev platform-images publish --revision
86d0c0f1dfad5cab56b333aad60fdb3c4a9f38af`. Zot read-back approved all four
immutable references and the local catalog was applied. The durable receipt is
under `.local/hephaestus/platform-images/installations/86d0c0f1dfad5cab56b333aad60fdb3c4a9f38af/`
as `review.json`, `catalog.json`, and `catalog-apply.json`.
