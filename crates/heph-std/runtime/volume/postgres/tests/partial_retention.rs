//! Worker metadata receipts and CAS, not physical custody or actor retirement.

#[path = "owned_registration/support.rs"]
#[allow(dead_code)] // Reuse original seal fixtures without rewriting 110 tests.
mod seals;

#[path = "owned_provisioning/support.rs"]
#[allow(dead_code)] // Reuse full 111 claims and metadata DTOs; no host proof implied.
mod backing;

mod partial_retention {
    mod fence;
    mod guards;
    mod progress;
    mod recording;
    mod support;
    mod upgrade;
}

#[path = "private_migrations/mod.rs"]
mod private_migrations;
