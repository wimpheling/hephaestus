//! Read-only comparison of a permanent original closure under fresh removal.
use crate::{
    CommandIdentity, DeploymentError, DeploymentIntent, DeploymentOperation,
    OriginalInstanceClosure, PreparedInstanceSafetyTarget,
};
use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;

/// Checked expected original data, never an execution or cleanup authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExistingInstanceClosureRequest {
    command: CommandIdentity,
    intent: DeploymentIntent,
    target: PreparedInstanceSafetyTarget,
    expected: OriginalInstanceClosure,
}
impl ExistingInstanceClosureRequest {
    /// Compares the current command and original closure to immutable intent pins.
    ///
    /// No admission, authentication or provider ownership follows from construction.
    ///
    /// # Errors
    /// Rejects a substituted target, original claim or non-distinct Remove command.
    pub fn new(
        command: CommandIdentity,
        intent: DeploymentIntent,
        target: PreparedInstanceSafetyTarget,
        expected: OriginalInstanceClosure,
    ) -> Result<Self, DeploymentError> {
        let original = target.original();
        let resource = intent
            .resources()
            .get(&original.resource)
            .ok_or(DeploymentError::IntentMismatch)?;
        expected.validate(original)?;
        if command.operation() != DeploymentOperation::Remove
            || command == expected.drain.command
            || intent.id() != original.deployment_id
            || intent.project_id() != target.prepared().project_id()
            || resource.identity() != original.identity
            || resource.input_hash() != original.input_hash
        {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(Self {
            command,
            intent,
            target,
            expected,
        })
    }
    /// Returns the new current manager command to independently authenticate.
    #[must_use]
    pub const fn command(&self) -> CommandIdentity {
        self.command
    }
    /// Returns the full checked intent to independently compare to storage.
    #[must_use]
    pub const fn intent(&self) -> &DeploymentIntent {
        &self.intent
    }
    /// Returns the original prepared target as comparison data.
    #[must_use]
    pub const fn target(&self) -> &PreparedInstanceSafetyTarget {
        &self.target
    }
    /// Returns original immutable closure data; it grants no execution permission.
    #[must_use]
    pub const fn expected(&self) -> &OriginalInstanceClosure {
        &self.expected
    }
}
/// Loads an unchanged original closure under a genuine fresh Remove admission.
///
/// Implementations require fresh ALL-owned cleanup permission and matched trusted
/// configuration, and independently compare original import, request and history
/// in one coherent read-only snapshot. No physical absence or cleanup is implied.
#[async_trait]
pub trait ExistingInstanceClosureRepository: Send + Sync + 'static {
    /// Rechecks immutable lineage and returns the original data without new effects.
    ///
    /// # Errors
    /// Denies unsupported configuration, permission loss, missing admission, changed
    /// originals or unknown/foreign history. Historical actor data is not credentials.
    async fn load_existing_instance_closure(
        &self,
        _identity: &AuthenticatedIdentity,
        _request: &ExistingInstanceClosureRequest,
    ) -> Result<OriginalInstanceClosure, DeploymentError> {
        Err(DeploymentError::InvalidAction)
    }
}
