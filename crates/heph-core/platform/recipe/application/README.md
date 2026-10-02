# Recipe deployment application contracts

`DeploymentIntent::new` checks that parsed declaration and resolved graph share
the same recipe identity, version, contract, and declaration hash. It freezes
canonical TOML, resolved JSON, ordinary inputs, exact external identities,
released authority ceilings, dependency order, retention intent, and predicted
resource IDs. UUID v5 resource identities depend only on deployment, resource
name, and purpose. Per-attempt request identity cannot change those IDs.

This crate performs no database, filesystem, VM, or network effects. Planning is
not authorization. A repository implementation must derive release and external
resource views from authoritative adapters, check live project and resource
permissions before replay or effects, audit those decisions, and commit progress
with project events and the existing product outbox. Recipe version one rejects
required secret slots during domain resolution; it stores no credentials or
provider handles.

`DeploymentRepository` admits install/remove, inspects recoverable partial state,
claims resource effects with compare-and-swap versions and fencing generations,
records definite or ambiguous outcomes, reconciles, and finishes commands. Each
claim commits before provider execution. Cancellation, crash, or expired claims
do not prove provider absence and cannot justify another creation or unsafe
storage reuse. Removal drains consumers and proves detach/fencing before deletion.
External volumes can only be verified; retain/delete policy never grants access.
Retained owned resources and removed deployment tombstones stay discoverable.

Validated intent has no deserializer. Database hydration must reparse canonical
declaration source, resolve against authoritative immutable historical release
and external facts, and compare the original fingerprints. Historical inspection
must remain possible after source revocation. Current publication, live resource
facts, and permissions are checked independently before new or resumed effects;
an old receipt cannot authorize them.

Logical command identity is actor/operation/idempotency scoped. Attempt provenance
records a distinct request and attempt ID. Changed input under either a logical
command or project-local deployment key is a conflict. Receipts identify committed
project events; provider errors are represented by closed safe diagnostic codes.
