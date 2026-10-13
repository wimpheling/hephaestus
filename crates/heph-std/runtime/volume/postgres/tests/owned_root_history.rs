//! Actual worker history queries against private111; no physical ownership proof.

#[path = "owned_provisioning/support.rs"]
#[allow(dead_code)] // Reuse actual purpose admission; observation fixtures remain unused.
mod backing;
#[path = "private_migrations/mod.rs"]
mod private_migrations;
#[path = "owned_registration/support.rs"]
#[allow(dead_code)] // Reuse real sealed actor/worker pools, not physical fixture assertions.
mod seals;

mod owned_root_history {
    mod lookup;
    mod support;
}
