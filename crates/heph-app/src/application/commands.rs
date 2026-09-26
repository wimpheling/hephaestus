//! Shared typed command application operations.

#[cfg(feature = "test-fixtures")]
mod barrier;
mod dispatch;
mod ids;
mod release;
mod secret;
mod state;
mod types;

#[cfg(feature = "test-fixtures")]
pub use barrier::{
    CreateUpdateAdmissionBarrier, CreateUpdateAdmissionBarrierGuard,
    install_create_update_admission_barrier, notify_reconciler_update_admission,
};
pub use dispatch::dispatch;
pub use state::InternalCommandState;
pub use types::{CapabilitySelectionInput, InternalCommand, RecoveryAction};
