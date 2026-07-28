# Identity and database-native authorization

Phase 3 makes PostgreSQL the production authority for identity, tenancy, and
authorization. There is no production tuple synchronization process and no
OpenFGA or SpiceDB runtime service.

## Request flow

1. OIDC middleware verifies the JWT signature, configured issuer, audience,
   expiry, and interactive-flow nonce.
2. The verified `(issuer, subject)` maps to exactly one active internal user.
3. The request opens one PostgreSQL transaction and sets
   `hephaestus.actor_id` and `hephaestus.request_id` with transaction-local
   `set_config`.
4. Commands invoke the typed `Authorizer` in that transaction.
5. The generated Mélange `check_permission` dispatcher evaluates the current
   transaction's domain-derived tuples.
6. Command authorization is written once to
   `authorization_audit_events`; ordinary RLS row filtering is not audited.
7. RLS independently constrains reads and writes by the normal application
   role.

Git is authorized before `git-http-backend` starts because PostgreSQL cannot
protect bare repositories on the filesystem. Clone and fetch require
`repository.can_read`; push requires `repository.can_write`. A successful push
persists actor and request provenance, and run-request creation additionally
checks `agent.can_execute` before writing its outbox command.

## Database roles

- `hephaestus_app` is `NOLOGIN NOBYPASSRLS`, does not own protected tables,
  and is the request-facing role.
- `hephaestus_authz_owner` is `NOLOGIN BYPASSRLS` and owns only the locked
  security-definer dispatcher wrapper. This prevents recursive RLS while the
  generated functions read `melange_tuples`.
- `hephaestus_worker` is `NOLOGIN BYPASSRLS` for trusted background
  orchestration and cleanup after a request has already produced a durable,
  authorized command.

Protected domain tables use `FORCE ROW LEVEL SECURITY` and define both
visibility (`USING`) and write (`WITH CHECK`) policies. Child audit tables such
as Git ref updates, run events, and volume leases inherit visibility through
their protected parent.

## Model and generation

The canonical model is [`../authz/hephaestus.fga`](../authz/hephaestus.fga).
See [`../authz/README.md`](../authz/README.md) for pinned Mélange/OpenFGA
versions, generation commands, drift checks, doctor checks, and compatibility
fixtures.
