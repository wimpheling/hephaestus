# Expand repository OCI builder lifecycle E2E coverage

Owner: unassigned

## Outcome

Extend the existing real-Zot repository-builder E2E harness beyond its current
publication, materialization, execution, and cross-project-denial coverage so
that lifecycle recovery and retirement behavior is proven independently.

## Dependencies

- The completed forge-owned registry blueprint in
  `tasks/done/forge-owned-oci-registry.md`.
- `scripts/test-repository-oci-builder-e2e.sh` and its authenticated disposable
  Zot/PostgreSQL fixture.

## Implementation checklist

- [ ] Add deterministic real-Zot E2E coverage for a publisher retry after an
  interrupted or failed publication, proving exactly one approved immutable
  result and no token disclosure.
- [ ] Add missing-approved-content coverage: remove or make the approved
  registry content unavailable, then prove materialization/execution fails
  closed with an operator-safe diagnostic.
- [ ] Add retirement coverage proving a retired builder cannot be selected or
  executed while historical immutable records remain auditable.
- [ ] Preserve and rerun the existing build, publication, verification,
  materialization, execution, and cross-project-denial path in the expanded
  harness.
- [ ] Run the focused E2E and the repository quality gate, recording the
  commands and results before moving this task to `tasks/done/`.

## Completion evidence

- [ ] Record the successful real-Zot E2E output for retry, missing-content,
  retirement, materialization/execution, and cross-project denial.
