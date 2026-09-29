# Image core

Image owns the catalog of OCI images that Hephaestus may select and the
application boundary that resolves an immutable image choice. It is separate
from Forge's build and registry publication workflows.

## Contracts and decisions

- [`domain/`](domain/) defines image keys and IDs, OCI references and digests,
  roles, availability, toolchains, provenance, and publication evidence.
- [`application/`](application/) defines the `ImageCatalog` and
  `RegistryPublicationCatalog` ports and validates selections before they enter
  build or runtime configuration.
- [`postgres/`](../../heph-std/image/postgres/) is the PostgreSQL catalog
  adapter. It stores catalog metadata and resolves publication evidence through
  the declared database boundary.

Selections carry an immutable reference and digest. Availability, role,
  provenance, signatures, SBOM, scan evidence, and platform policy version are
  part of the catalog contract. Image selection is deliberately independent of
  guest execution policy: choosing an image does not grant network, secret,
  state-volume, mount, or resource permissions.

## Boundaries

Image does not build an OCI image, run a VM, or publish to a registry. Forge
build and registry contexts own those workflows; concrete workers and registry
clients live under [`heph-std/forge/`](../../heph-std/forge/), and
[`heph-app`](../../heph-app/) wires the catalog into RPC, build, and startup
flows. Schema DDL remains in the root [`migrations/`](../../../migrations/)
directory.

See [`docs/repository-image-builds.md`](../../../docs/repository-image-builds.md)
for the repository-image workflow and [`docs/application.md`](../../../docs/application.md)
for composition boundaries.
