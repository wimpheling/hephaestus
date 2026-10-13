//! Actual sealed provisioning metadata tests with worker fixture observations.
//! These tests prove database fences and atomicity, not physical backing birth.

#[path = "owned_registration/support.rs"]
#[allow(dead_code)] // Reuse original seal fixtures and role pools without altering 110.
mod seals;

mod owned_provisioning {
    mod admission;
    mod guards;
    mod observations;
    mod support;
    mod upgrade;
}

#[path = "private_migrations/mod.rs"]
mod private_migrations;
