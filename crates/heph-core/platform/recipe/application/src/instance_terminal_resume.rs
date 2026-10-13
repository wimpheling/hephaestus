//! Native143 comparison projections; no constructor authenticates or observes IO.
mod delete;
mod detach;
mod progress;
mod publisher;
use crate::{CommandIdentity, DeploymentError, DeploymentIntent, PreparedInstanceDeleteRequest};
use async_trait::async_trait;
use capability_domain::CapabilitySlotKey;
pub use delete::completed::PreparedInstanceDeleteCompletedHistory;
pub use delete::{PreparedInstanceDeleteHistory, PreparedInstanceDeleteHistoryState};
pub use detach::PreparedInstanceDetachRecovery;
use identity_domain::AuthenticatedIdentity;
pub use progress::PreparedInstanceTerminalProgress;
pub use publisher::PreparedInstanceTerminalPublisher;

/// Optional exact history port, separate from physical ownership and live authority.
///
/// Implementations use their original configured pools and protected native143
/// readers. Complete Create/closure/actual completed Detach/configuration and
/// terminal/event/progress history must be qualified independently of current
/// mutable instance state. Saved publishers are comparison data, never credentials.
#[async_trait]
pub trait PreparedInstanceTerminalHistoryRepository: Send + Sync + 'static {
    /// Loads exact original Delete comparison data under current admitted Remove.
    ///
    /// Original before CAS must come from positively observed native history, not
    /// current Removed version. A historical unwritten endpoint proves no absence.
    ///
    /// # Errors
    /// Denies unsupported adapters, current rights loss, missing/unknown originals,
    /// foreign configuration, partial publication or conflicting history.
    async fn load_delete_continuation(
        &self,
        _identity: &AuthenticatedIdentity,
        _request: &PreparedInstanceDeleteRequest,
    ) -> Result<PreparedInstanceDeleteHistory, DeploymentError> {
        Err(DeploymentError::InvalidAction)
    }
    /// Loads positively inactive Applied Delete for the actual original Remove.
    ///
    /// Repeated original-command delivery may have no active claim. The adapter
    /// must independently read the whole stored intent/preparation and positively
    /// qualified terminal original/Applied transition/worker/event history under
    /// current ALL-owned rights and its retained complete configuration. Original
    /// Delete.command must equal current_command. Distinct managers must use the
    /// genuine fresh admission/history path instead; no claim or CAS is inferred.
    /// Returned comparison data still requires independent worker observation.
    ///
    /// # Errors
    /// Denies unsupported adapters, lost current rights, wrong original command,
    /// foreign scope or absent, active, unproven or conflicting completed history.
    async fn load_original_delete_history(
        &self,
        _identity: &AuthenticatedIdentity,
        _intent: &DeploymentIntent,
        _current_command: CommandIdentity,
        _resource: &CapabilitySlotKey,
    ) -> Result<PreparedInstanceDeleteHistory, DeploymentError> {
        Err(DeploymentError::InvalidAction)
    }
    /// Observes unique actual Applied Delete and its immutable terminal record.
    ///
    /// Dedicated worker qualification must compare full original claim, committed
    /// transition/verification/event/fingerprint and exact seven-field State. It
    /// produces neither a worker receipt nor a physical cleanup certificate.
    ///
    /// # Errors
    /// Denies unsupported roles/configuration or absent, partial or contradictory
    /// Applied history. No inactive claim or tombstone alone establishes success.
    async fn observe_completed_delete(
        &self,
        _request: &PreparedInstanceDeleteRequest,
    ) -> Result<PreparedInstanceDeleteCompletedHistory, DeploymentError> {
        Err(DeploymentError::InvalidAction)
    }
}
