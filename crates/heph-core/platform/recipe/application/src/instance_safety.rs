//! Read-only original instance creation and closed-runtime comparison data.
//!
//! Scope labels come from trusted actual provider composition. These DTOs grant
//! neither provider ownership nor permission to issue runtime effects.

mod inventory;
mod target;

pub use inventory::{InstanceSafetyInventory, InstanceSafetyWork, InstanceSafetyWorkKind};
pub use target::PreparedInstanceSafetyTarget;

use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use release_domain::InstanceRemovalId;

use crate::{CommandIdentity, DeploymentError};

/// Action-specific observation, separate from installation and activation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreparedInstanceSafetyState {
    /// Positive original creation has not been observed; no absence is asserted.
    Unproven,
    /// Exact committed creation exists, without qualified permanent closure.
    Created,
    /// Permanent closure exists but complete runtime safety remains unproven.
    ClosedHeld,
    /// Complete database cleanup evidence matches the configured scope.
    ///
    /// A no-Run result proves the observation boundary, not physical VM cleanup.
    Drained,
}

/// Exact backend observation; the owning adapter must establish its evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInstanceSafetyObservation {
    /// Immutable original target, compared independently by the worker adapter.
    pub target: PreparedInstanceSafetyTarget,
    /// Positive committed original creation, closure and cleanup classification.
    pub state: PreparedInstanceSafetyState,
    /// Qualified immutable removal request, never inferred from a closed gate.
    pub removal_id: Option<InstanceRemovalId>,
    /// Complete global comparison inventory; a host filter cannot prove absence.
    pub inventory: InstanceSafetyInventory,
}

/// Current-manager inspection and independent worker observation only.
///
/// Current Remove may read original Install data without borrowing its identity
/// or Source grants. Neither method mutates closure, records outcomes, drains a
/// VM, activates an instance or grants volume access. Historical uncertainty stays
/// held; unsupported adapters must never fall back to import replay.
#[async_trait]
pub trait PreparedInstanceSafetyRepository: Send + Sync + 'static {
    /// Inspects the exact original target after fresh current Remove authorization.
    ///
    /// # Errors
    /// Denies unsupported adapters, inactive managers, changed input or config,
    /// missing lineage, and any owned-resource cleanup permission denial.
    async fn inspect_for_remove(
        &self,
        _identity: &AuthenticatedIdentity,
        _command: CommandIdentity,
        _target: &PreparedInstanceSafetyTarget,
    ) -> Result<PreparedInstanceSafetyObservation, DeploymentError> {
        Err(DeploymentError::InvalidAction)
    }

    /// Independently reads exact immutable birth and global cleanup evidence.
    ///
    /// # Errors
    /// Denies unsupported adapters, actor pools, contradictory original pins,
    /// configuration drift or unavailable persistence. Scope data is no receipt.
    async fn observe_safety(
        &self,
        _target: &PreparedInstanceSafetyTarget,
    ) -> Result<PreparedInstanceSafetyObservation, DeploymentError> {
        Err(DeploymentError::InvalidAction)
    }
}
