# Purpose

This directory contains the PostgreSQL control-plane provider. It gives
application and worker workflows durable access to repository metadata while
keeping database roles, authorization context, and artifact storage at the
standard adapter boundary.

# Responsibilities

The provider connects release, build, project, instance, repository, and run
workflows to their persisted records. It also resolves launch contracts and
artifact streams, so a caller receives validated control-plane state instead of
raw database rows.

`OrganizationApplication` serves authenticated, transaction-local RLS queries
for paginated organization summaries and their related project and repository
metadata.

# When

Use this provider when composing the control-plane services. Select an
application pool for user-facing reads, a worker pool for worker-owned paths,
and the repository adapter methods when a workflow needs an authorized,
transactionally consistent control-plane result.
