//! Actual `PostgreSQL` evidence with controlled provider IO, not physical VM proof.
#[path = "profiled_lifecycle/fixture.rs"]
// Reuse the scalar graph; unrelated lifecycle cases are unused.
#[allow(dead_code)]
mod base;
#[path = "strict_legacy/controlled.rs"]
mod controlled;
#[path = "strict_legacy/evidence.rs"]
mod evidence;
#[path = "legacy_vm_placement/fixture.rs"]
// Reuse exact birth provenance; update seed helper is unused here.
#[allow(dead_code)]
mod fixture;
#[path = "strict_legacy/provider.rs"]
mod provider;
#[path = "cleanup/fixture.rs"]
// Existing public migration/role helpers also contain unrelated assertions.
#[allow(dead_code)]
mod public_fixture;
#[path = "planned_creation/seed.rs"]
// The shared fixture references genuine update provenance; no update mutation here.
#[allow(dead_code)]
mod seed;
#[path = "postgres/support.rs"]
// Reuse genuine Git request pins; unrelated transport/retry helpers remain unused.
#[allow(dead_code)]
mod support;
#[path = "strict_legacy/volumes.rs"]
mod volumes;
