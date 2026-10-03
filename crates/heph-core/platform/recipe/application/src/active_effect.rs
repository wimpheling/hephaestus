//! Backend-only authoritative recovery of a durable active effect.

use async_trait::async_trait;
use capability_domain::CapabilitySlotKey;
use identity_domain::AuthenticatedIdentity;

use crate::{CommandIdentity, DeploymentError, DeploymentId, EffectClaim};

/// Reads exact durable claims after fresh command and current authority checks.
///
/// Install readers must revalidate the original actor, sole original admission,
/// complete execution preparation, configured policy and live source/resource
/// rights. An admitted current manager removal may read the original installation
/// claim under cleanup authority without continued source access. Historical
/// actor data must never be converted into an authenticated identity.
///
/// Returned data conveys no permission to invoke a provider and no outcome proof.
/// Transactions lock project, deployment and resource in that order. The original
/// claim version and request remain immutable even when ambiguity advances current
/// progress. Absence is returned only for a genuinely inactive resource; orphaned
/// or contradictory persisted claims remain held for reconciliation.
#[async_trait]
pub trait ActiveDeploymentEffectRepository: Send + Sync + 'static {
    /// Loads the full current active claim for an admitted current command.
    ///
    /// # Errors
    /// Rejects borrowed/unadmitted commands, live denial or configuration drift.
    /// Contradictory persisted lineage requires reconciliation; missing resources
    /// or deployments are unavailable and never treated as a claim absence.
    async fn load_active_effect(
        &self,
        identity: &AuthenticatedIdentity,
        current_command: CommandIdentity,
        deployment: DeploymentId,
        resource: &CapabilitySlotKey,
    ) -> Result<Option<EffectClaim>, DeploymentError>;
}
