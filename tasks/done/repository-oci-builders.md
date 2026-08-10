# Repository OCI Builder Specification

## Purpose

Repositories may define Dockerfile-based builder roots for their own agents.
The platform builds these OCI images in isolation, verifies them, materializes
the approved immutable digest as a VM root, and only then lets an agent build
select it. This is a builder-root contract; it never changes an agent's
separate runtime `root_image` contract.

## Source contract

A repository can commit one root-level `heph.builders.toml` file:

```toml
version = 1

[[builders]]
key = "typescript-tools"
display_name = "TypeScript tools"

[builders.oci]
dockerfile = "containers/typescript-tools.Dockerfile"
context = "."
base = "typescript-node-ubuntu"
```

- `key` is a repository-local lowercase identifier, stable across revisions.
- `dockerfile` and `context` are safe paths relative to the exact source
  commit; contexts outside the repository, remote URLs, and symlink escapes
  are rejected.
- `base` is an approved platform catalog key, never a tag or arbitrary OCI
  reference. It resolves to a ready, available, digest-pinned catalog image
  when preparation is requested.
- A repository owns its builders. An `agent.toml` in that repository may select
  only its repository's builder. Explicit cross-repository sharing is deferred.

## Dockerfile contract

The preparation worker exposes the resolved platform image as the reserved
Dockerfile name `heph-base`. The first stage must use it:

```dockerfile
FROM heph-base AS build
```

Later stages may use a previous named stage or `scratch`. Any other `FROM`
reference is rejected before OCI execution, so a repository cannot pull an
unapproved base image. Dockerfile `ADD`/`COPY` remote URLs are rejected.

The build context is the exact checked-out commit and declared `context` path.
No host paths, daemon socket, ambient credentials, or tenant secrets are
mounted. Network access is disabled initially; a future policy may permit a
bounded registry-egress profile only when both the platform base and project
policy allow it.

## Agent selection and immutable resolution

An agent selects a repository builder by key:

```toml
[build.builder]
kind = "repository"
key = "typescript-tools"
```

The legacy exact `build.root_image` and platform-key selection remain valid.
For a repository selection, the receive/manual-build transaction resolves the
definition at the exact source commit and records the resulting immutable OCI
digest on the build request. A source/config/base-policy change creates a new
builder revision; old successful revisions remain auditable but are never
silently replaced.

## Preparation lifecycle

1. Parse and validate `heph.builders.toml` during the receive workflow.
2. Resolve the catalog base to a ready, available digest and queue an isolated
   OCI preparation job for the exact source revision/context digest.
3. The worker builds without ambient credentials, scans the OCI output, writes
   provenance/SBOM references, and records an immutable output digest.
4. Only a successful scan and verified provenance make the revision `ready`.
5. A materializer exports that exact digest into the daemon's explicit
   digest-to-rootfs manifest. Only a ready, materialized revision can run.

## Single-node worker configuration

The local daemon enables repository OCI preparation only when
`HEPHAESTUS_OCI_BUILDER_ROOTFS_ROOT` is set. It then requires an explicit
administrator-owned base-layout manifest and private checkout/output roots:

```text
HEPHAESTUS_OCI_BUILDER_ROOTFS_ROOT=/var/lib/hephaestus/builder-rootfs
HEPHAESTUS_OCI_BUILDER_BASE_LAYOUT_MANIFEST=/etc/hephaestus/builder-bases.json
HEPHAESTUS_OCI_BUILDER_CHECKOUT_ROOT=/var/lib/hephaestus/builder-checkouts
HEPHAESTUS_OCI_BUILDER_OUTPUT_ROOT=/var/lib/hephaestus/builder-oci
HEPHAESTUS_OCI_BUILDER_IMAGE_REFERENCE_PREFIX=local.hephaestus/builders
```

The base-layout manifest is a non-empty JSON object mapping the approved,
digest-pinned catalog reference to an absolute OCI-layout directory. It is
separate from the VM root manifest: Buildah consumes OCI layouts, while the VM
only receives the exported root filesystem. The daemon also accepts explicit
absolute paths for Git, Tar, Buildah, Trivy, and Umoci; their defaults are
`/usr/bin/{git,tar,buildah,trivy,umoci}`. A deployment must install those
tools, make the rootfs root an allowed libkrun image root, and provision the
base layouts before enabling the worker.

For each durable job, Git archives precisely the recorded commit to a private
directory, Buildah builds rootlessly with `--pull=never` and `--network=none`,
and exports an OCI layout. The worker creates a temporary OCI archive only for
Trivy's offline image scan, writes scan and provenance records beside the
layout, removes the source checkout and scan archive, and uses Umoci to unpack
the verified layout. No repository-controlled executable, path, network,
credential, socket, or registry setting becomes a host capability.

Preparation states are `draft`, `preparing`, `ready`, `failed`, and `retired`.
A failed immutable revision may be retried; changing source metadata creates a
new revision instead of mutating prior provenance.

## UI and events

The repository Builders page is a read-only view of committed definitions and
revisions. It shows the resolved base/output digests and preparation state;
creation is intentionally not a UI mutation. Every worker lifecycle transition
emits the project product event used by connected clients, which reload the
typed snapshot rather than polling. An authorized retry operation remains a
follow-up because a retry must preserve the same immutable revision and never
accept caller-supplied output metadata.

## Initial non-goals

- Cross-repository builder sharing.
- Arbitrary registry pulls, remote Dockerfile contexts, build secrets, or host
  Docker/Podman socket access.
- Docker Compose, unpinned base tags, and mutable image execution.
