# Purpose

`recipe-domain` parses bounded resource recipe TOML and resolves a static graph
against ordinary inputs, exact external volume references, and an authoritative
published-release catalog. It provides reusable intent for distribution recipes;
it does not install workloads or grant permissions.

# Responsibilities

Version one accepts named volumes and instances, typed scalar inputs/defaults,
explicit dependencies and volume-slot bindings, declared outputs, and removal
policy. It rejects unknown fields, unsupported syntax/kinds/versions, cycles,
resource aliases, invalid capacities, and changes under an existing recipe
identity/version. Canonical JSON freezes declaration and resolved snapshot
identity. The caller must check identity against authoritative persisted records.
Each volume may satisfy only one instance slot across the entire graph, including
read-only attachments. Shared reads need a future explicit consistency contract.

Release IDs and export IDs are exact pins. Resolution checks their association
and publication through an application-supplied catalog, then checks required
slots, exact guest paths/modes, minimum capacity, and parameter semantics.
The catalog is trusted application data, never caller-supplied transport data.
External identities/capacities have the same authoritative-source requirement.
The application derives required-secret counts from the released secret schema;
nonzero counts fail closed before attachment compatibility resolution.
No secret inputs, provider paths/credentials, SQL, host hooks, or expressions
are supported. External volumes always retain; created durable volumes retain
by default. Referenced resources in outputs convey identity, not authority.

# When

Use `parse_recipe` for source validation, `validate_identity` against a recorded
identity/version, then `ValidatedRecipe::resolve` for immutable resolved intent.
The checked [local SQLite example](examples/local-sqlite.toml) composes a pinned
application with a volume; SQLite lives in the application and uses file bytes.
The placeholder release must be present and published in the supplied catalog.

Application orchestration still needs live caller authorization, exact consumer
grants, stable project ownership, idempotent durable deployment records and
provider reconciliation. It must enforce provider capacity limits, validate
actual volume-ID bindings, drain/detach/fence before removal, and keep retained
data discoverable. This crate supplies neither runtime revocation nor writer
fencing. Install/inspect/remove effects, service endpoints, secrets, upgrades,
and data migrations remain outside this initial pure contract slice.
