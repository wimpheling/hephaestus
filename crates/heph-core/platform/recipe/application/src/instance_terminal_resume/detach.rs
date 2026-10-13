use super::PreparedInstanceTerminalProgress;
use crate::{DeploymentError, PreparedInstanceDetachRequest, RemovalProgress};

/// Exact instance Owned/Delete-policy Detach Recovery comparison, not a receipt.
///
/// The checked request retains full intent/Create/tagged closure/config/original
/// Detach. Native143 independently qualifies its actual completed Drain predecessor
/// and complete contiguous active history; this model cannot mint that evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInstanceDetachRecovery {
    request: PreparedInstanceDetachRequest,
    observed: PreparedInstanceTerminalProgress,
}
impl PreparedInstanceDetachRecovery {
    /// Compares full checked originals with exact advanced same-attempt progress.
    ///
    /// # Errors
    /// Rejects non-RecoveryRequired progress, generation gaps, substituted active
    /// attempt/prior action or an endpoint not advanced beyond original Detach.
    pub fn new(
        request: PreparedInstanceDetachRequest,
        observed: PreparedInstanceTerminalProgress,
    ) -> Result<Self, DeploymentError> {
        observed.active(request.effect())?;
        request.intent().resources()[&request.effect().resource]
            .validate_action(crate::ResourceAction::Delete)?;
        if observed.progress().removal != RemovalProgress::RecoveryRequired
            || request.closure().drain.generation.checked_add(1)
                != Some(request.effect().generation)
            || request
                .closure()
                .drain
                .resource_version
                .checked_add(2)
                .is_none_or(|version| request.effect().resource_version < version)
        {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(Self { request, observed })
    }
    /// Returns full originals and current command, none of which grants authority.
    #[must_use]
    pub const fn request(&self) -> &PreparedInstanceDetachRequest {
        &self.request
    }
    /// Returns all seven observed fields, not a contiguous-history certificate.
    #[must_use]
    pub const fn observed_progress(&self) -> &PreparedInstanceTerminalProgress {
        &self.observed
    }
}
