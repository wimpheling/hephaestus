//! Actual private profile lifecycle SQL and controlled production-core proof.
//!
//! This target does not certify physical VM destruction or app activation.
//! Its private loader must become an ordinary migrator before publication.

#[path = "profiled_lifecycle/controlled.rs"]
mod controlled;
#[path = "profiled_lifecycle/controlled_volumes.rs"]
mod controlled_volumes;
#[path = "profiled_lifecycle/failed_provider.rs"]
mod failed_provider;
#[path = "profiled_lifecycle/fixture.rs"]
mod fixture;
#[path = "profiled_lifecycle/guards.rs"]
mod guards;
#[path = "profiled_lifecycle/legacy.rs"]
mod legacy;
#[path = "profiled_lifecycle/owned.rs"]
mod owned;
#[path = "cleanup/fixture.rs"]
// Existing checksum/role/migration helpers include unrelated cleanup cases.
#[allow(dead_code)]
mod public_fixture;
#[path = "planned_creation/seed.rs"]
// Reuse the genuine manager/update/candidate graph; other seeds are unused.
#[allow(dead_code)]
mod seed;
#[path = "postgres/support.rs"]
// Reuse exact Git producer graph; unrelated mailbox/retry helpers are unused.
#[allow(dead_code)]
mod support;
#[path = "profiled_lifecycle/update.rs"]
mod update;
