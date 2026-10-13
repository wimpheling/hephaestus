//! Genuine metadata lineage tests against disposable `PostgreSQL`.
//! These tests prove database atomicity, not backing-file creation ownership.

mod owned_registration {
    mod behavior;
    mod guards;
    mod support;
    mod upgrade;
}

#[path = "private_migrations/mod.rs"]
mod private_migrations;
