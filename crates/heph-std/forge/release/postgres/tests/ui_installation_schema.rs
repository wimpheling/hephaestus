//! Real-role schema coverage for migration 0088 UI installations.
//!
//! Run with `HEPHAESTUS_POSTGRES_TEST_URL` and let the validation runner require
//! the `REAL_RELEASE_UI_INSTALLATION_SCHEMA=1` marker.

#[path = "ui_installation_schema/barrier.rs"]
mod barrier;
#[path = "ui_installation_schema/barrier_locks.rs"]
mod barrier_locks;
#[path = "ui_installation_schema/global.rs"]
mod global;
#[path = "ui_installation_schema/lifecycle.rs"]
mod lifecycle;
#[path = "ui_installation_schema/support/mod.rs"]
mod support;
