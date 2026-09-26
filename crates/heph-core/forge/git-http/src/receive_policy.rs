//! Trusted receive-policy enforcement before canonical ref mutation.
//!
//! This module deliberately accepts already-inspected receive facts. Producing
//! those facts safely requires a quarantined pack and trusted repository
//! inspection; parsing client claims is not sufficient. The caller must invoke
//! [`authorize_before_canonical_mutation`] before making proposed objects or
//! refs canonical.

mod authorization;
mod context;
mod errors;
mod proposal;

pub use authorization::{
    CapabilityReceivePolicyGuard, ReceiveMutationPermit, ReceivePolicyGuard,
    authorize_before_canonical_mutation,
};
pub use context::ResolvedRuntimeReceiveContext;
pub use errors::{GuardedReceiveError, ReceivePolicyError};
pub use proposal::{TrustedPathChange, TrustedReceiveProposal, TrustedReceiveUpdate};

#[cfg(test)]
#[path = "receive_policy/tests.rs"]
mod tests;
