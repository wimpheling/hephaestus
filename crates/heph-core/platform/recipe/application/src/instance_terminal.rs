//! Terminal comparison inputs, independent of admission and physical ownership.
//!
//! Implementations must qualify complete original history and current rights.
//! No constructor here observes a database, creates a worker receipt or performs IO.

mod observations;
mod requests;

pub use observations::{PreparedInstanceDeleteObservation, PreparedInstanceDetachObservation};
pub use requests::{PreparedInstanceDeleteRequest, PreparedInstanceDetachRequest};

use crate::DeploymentError;
use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;

/// Optional protected prepared-instance terminal boundary.
///
/// Unsupported adapters deny every operation. Implementations retain the actual
/// configured worker/application pools and owning physical composition. Request
/// values, observations and historical identities never authorize an effect.
#[async_trait]
pub trait PreparedInstanceTerminalRepository: Send + Sync + 'static {
    /// Reads complete original closure and post-physical attachment evidence.
    ///
    /// The dedicated worker must compare every original pin and inventory member;
    /// a closed gate, no Run rows or caller-supplied Drained state is insufficient.
    /// No generic Applied receipt or recipe progress is written by this operation.
    ///
    /// # Errors
    /// Denies unsupported roles/configuration, unknown or conflicting history,
    /// changed claims, incomplete physical cleanup and unavailable persistence.
    async fn observe_detach(
        &self,
        _request: &PreparedInstanceDetachRequest,
    ) -> Result<PreparedInstanceDetachObservation, DeploymentError> {
        Err(DeploymentError::InvalidAction)
    }

    /// Writes a protected logical tombstone under actual current Remove authority.
    ///
    /// Independently qualifies genuine admitted current command, ALL-owned rights,
    /// exact active Delete claim, committed completed/reconciled Detach, original
    /// closure, complete physical history and explicit instance CAS. Immutable
    /// history remains. A request contains no caller-created Detach proof.
    /// The committed terminal record must bind old Create, closure, real Detach,
    /// Delete/config/current publisher, before/after versions and genuine event.
    /// Returned comparison data is not the subsequent worker observation or receipt.
    ///
    /// # Errors
    /// Denies unsupported adapters, current authority loss, missing or conflicting
    /// lineage, stale CAS, incomplete cleanup and uncertain persistence outcomes.
    async fn delete_for_remove(
        &self,
        _identity: &AuthenticatedIdentity,
        _request: &PreparedInstanceDeleteRequest,
    ) -> Result<PreparedInstanceDeleteObservation, DeploymentError> {
        Err(DeploymentError::InvalidAction)
    }

    /// Independently qualifies the exact committed native tombstone and history.
    ///
    /// Worker observation must check the entire terminal record/event/claim/config
    /// graph and exact original closure/Detach before returning comparison data.
    /// Missing current work or a Removed state alone cannot establish completion.
    /// No actor authority, generic receipt or new effect is produced.
    ///
    /// # Errors
    /// Denies unsupported roles/configuration, absent/partial or conflicting native
    /// records, changed original pins and unavailable persistence.
    async fn observe_deleted(
        &self,
        _request: &PreparedInstanceDeleteRequest,
    ) -> Result<PreparedInstanceDeleteObservation, DeploymentError> {
        Err(DeploymentError::InvalidAction)
    }
}
