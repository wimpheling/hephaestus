//! Actual metadata/admission proof only, with no host IO or app activation.

#[path = "profiled_lifecycle/fixture.rs"]
// Reuse the checked scalar graph; unrelated lifecycle helpers are unused.
#[allow(dead_code)]
mod base;
#[path = "legacy_vm_placement/fixture.rs"]
mod fixture;
#[path = "legacy_vm_placement/guards.rs"]
mod guards;
#[path = "legacy_vm_placement/inventory.rs"]
mod inventory;
#[path = "legacy_vm_placement/normal.rs"]
mod normal;
#[path = "legacy_vm_placement/precreated.rs"]
mod precreated;
#[path = "cleanup/fixture.rs"]
// Reuse public migration and role helpers, without certifying cleanup here.
#[allow(dead_code)]
mod public_fixture;
#[path = "legacy_vm_placement/races.rs"]
mod races;
#[path = "planned_creation/seed.rs"]
// Reuse actual manager/update provenance, not synthetic authentication.
#[allow(dead_code)]
mod seed;
#[path = "postgres/support.rs"]
// The original graph also supports unrelated retry and mailbox cases.
#[allow(dead_code)]
mod support;
