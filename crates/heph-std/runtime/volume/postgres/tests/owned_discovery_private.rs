//! Actual original 110/111 database discovery; modeled worker observations are not filesystem proof.

#[path = "private_migrations/mod.rs"]
mod private_migrations;
#[path = "owned_provisioning/support.rs"]
#[allow(dead_code)] // Reuse the existing modeled worker observation constructor, not host evidence.
mod provisioning;
#[path = "owned_registration/support.rs"]
#[allow(dead_code)]
// Reuse immutable birth and role fixtures without changing their existing target.
mod seals;

mod owned_discovery {
    mod contradictions;
    mod fixture;
    mod originals;
}
