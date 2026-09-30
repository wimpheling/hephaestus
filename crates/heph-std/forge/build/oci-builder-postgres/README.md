# Purpose

`oci-builder-postgres` stores the durable job and publication state consumed by
the OCI build workers. It lets workers claim production or materialization
work, record immutable output and provenance, and resume an interrupted
registry publication without creating a second intent.

# Responsibilities

`PgOciImageProductionJobStore` maps PostgreSQL rows to the `heph-build` job
contracts, applies lease-based claims, and records success or bounded failure
for both production and materialization. `PgRepositoryOciImagePublicationStore`
checks the project and image owner, creates or resumes the exact registry
intent, and records verified publication evidence before approval.

The adapter validates source paths, image references, digests, output
provenance, lease durations, and failure reasons at the database boundary.
Its durable state gives workers idempotent retries while retaining the exact
project, repository, image, commit, and publication identities.

# When

Construct `PgOciImageProductionJobStore` and
`PgRepositoryOciImagePublicationStore` with the application pool, then inject
them into the OCI production worker and publication workflow. A worker calls
`claim_production` or `claim_materialization`, completes the result, and uses
the publication store for digest verification.
