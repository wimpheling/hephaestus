# Purpose

`authz-postgres` evaluates Hephaestus authorization decisions in PostgreSQL.
It gives Git transport and application code one provider implementation of the
core authorization ports, with request actor provenance attached to the same
database transaction as the decision.

# Responsibilities

`PostgresGitAuthorizer` maps Git reads and writes to the canonical permissions,
checks them through the Mélange database evaluator, and records the decision
with the request and authorization model version before committing. The actor
transaction helpers set the user, subject, request, and occurrence context;
runtime helpers set an exact run subject for already-authenticated workers.

# When

Construct `PostgresGitAuthorizer` with the authorization `PgPool` and pass it to
the Git transport at each repository request. Use `begin_actor_transaction` for
application work that must evaluate permissions with the same authenticated
actor context.
