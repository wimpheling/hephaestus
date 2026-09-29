# Purpose

`heph-build` coordinates the contracts that turn one repository commit into
an immutable OCI image and then into a materialized root usable by a daemon.
It carries exact source paths, build provenance, scan evidence, and durable
worker jobs between the build and publication workflow.

# Responsibilities

`RepositoryOciImageSourcePath::parse` keeps Dockerfile and context paths inside
the exact checkout. A claimed production job records the repository, commit,
context digest, approved base, and paths; completion must provide an immutable
image digest, attestation, scan result, and optional SBOM. The job store makes
production and materialization retryable through leases and explicit success
or failure records.

Registry publication uses a durable intent: the worker begins or resumes the
intent, records read-back verification, and only then reaches approval. This
keeps an image usable by later runtime selection only after its immutable
output and supply-chain evidence agree.

# When

Use this crate from a build worker that claims a production job, validates its
source paths and provenance, completes the image job, and then claims the
materialization job. The worker uses `OciImageProductionJobStore` and
`RepositoryOciImagePublicationStore` for those transitions.
