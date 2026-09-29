//! Fork-phase production Git evidence for the session-chat composed test.
//!
//! The composed browser acceptance uses this module for production fork
//! publication and fresh target provisioning.

#[path = "fork/helpers.rs"]
mod helpers;
#[path = "fork/provisioning.rs"]
mod provisioning;
#[path = "fork/publication.rs"]
mod publication;
#[path = "fork/state.rs"]
mod state;

pub use provisioning::provision_target;
pub use publication::exercise;
pub use state::{ForkTargetProvisioningState, ForkTargetState, SourceSessionState};
