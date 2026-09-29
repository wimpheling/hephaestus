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
