//! Authorized run reads and durable control request creation.

#[path = "run/admissions.rs"]
mod admissions;
#[path = "run/artifact/preview.rs"]
mod artifact;
#[path = "run/control.rs"]
mod control;
#[path = "run/model.rs"]
mod model;
#[cfg(test)]
#[path = "run/preview_tests.rs"]
mod preview_tests;
#[path = "run/queries.rs"]
mod queries;
#[cfg(test)]
#[path = "run/tests/reconciliation.rs"]
mod reconciliation_tests;
#[path = "run/view.rs"]
mod view;

pub use admissions::{
    PendingUpdateAdmission, is_update_hook_run, load_vm_launch_contract, pending_update_admissions,
    recoverable_update_hook_run_ids,
};
pub use model::*;
