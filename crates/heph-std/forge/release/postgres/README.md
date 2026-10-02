# Purpose

`release-postgres` is the PostgreSQL adapter for release publication, reusable
agent instances, capability bindings, repository attachments, update hooks,
and the control-plane UI installation state. It turns provider-neutral release
commands into authorized durable transactions consumed by run and gateway
workflows.

# Responsibilities

The adapter validates and persists release manifests, exact revisions,
immutable artifact references, typed capability ceilings, Git policies,
brokered-secret rules, gateway declarations, and update candidates. Every
command establishes actor context, checks current authorization and optimistic
state, records an event or outbox effect, and commits related rows together.
Update preparation and recovery preserve the old revision until the candidate
hook and its resource cleanup are resolved. UI installation and browser
resource queries apply the same tenant and generation checks, keeping a stale
or unauthorized revision from becoming active.

# When

Compose `ReleaseService` with the application command and storage adapters when
a release or instance command reaches the control plane:

```rust
let service = ReleaseService::new(pool, authorizer);
```

Call the service inside the command workflow and use its committed result or
recovery decision to drive run, gateway, and event publication.

## Explicit volume compatibility

Authored volume slots are validated, sorted, and frozen into the runtime contract
before its content hash is computed. Empty declarations omit the field and retain
the exact legacy JSON/hash. Legacy state requirements remain separate provenance
and are lifted only in typed effective views; published rows are not rewritten.

Publication accepts these declarations. Until exact instance bindings can be
materialized, legacy import, update candidate creation, and update hook admission
reject nonempty declarations after live project/source authorization and before
replay or metadata effects. Malformed persisted declarations fail closed. Absent
or empty declarations retain the legacy paths.

Recipe installation and normal import need subsequent binding/materialization
work before explicit-slot instances can run. Runtime mount conflict/symlink checks,
exact consumer grants, writer detach/fencing, and revocation remain separate work.
