//! Initial terminal comparison preparation; neither a receipt nor effect authority.
use crate::{
    CommandIdentity, DeploymentError, DeploymentExecutionProfile, DeploymentIntent,
    DeploymentOperation, EffectClaim, PreparedInstanceDeleteRequest, PreparedInstanceDetachRequest,
    PreparedInstanceSafetyTarget, RemoveResumeConfiguration, ResourceAction, ResourceOwnership,
};
use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;

/// Whole checked inputs before the authoritative original closure and Delete CAS are read.
///
/// Private fields prevent substituting originals after checking. Construction does
/// not authenticate any principal, admit an effect, infer cleanup or observe CAS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInstanceTerminalPreparationInput {
    intent: DeploymentIntent,
    target: PreparedInstanceSafetyTarget,
    effect: EffectClaim,
    current_command: CommandIdentity,
    configuration: RemoveResumeConfiguration,
}
impl PreparedInstanceTerminalPreparationInput {
    /// Retains the exact Create/import, Detach-or-Delete claim and current Remove.
    ///
    /// # Errors
    /// Rejects foreign or changed plan/scope, unsupported actions, invalid fencing
    /// provenance and reused Create attempt identities.
    pub fn new(
        intent: DeploymentIntent,
        target: PreparedInstanceSafetyTarget,
        effect: EffectClaim,
        current_command: CommandIdentity,
        configuration: RemoveResumeConfiguration,
    ) -> Result<Self, DeploymentError> {
        let original = target.original();
        let plan = intent
            .resources()
            .get(&original.resource)
            .ok_or(DeploymentError::IntentMismatch)?;
        if !matches!(
            effect.action,
            ResourceAction::Detach | ResourceAction::Delete
        ) {
            return Err(DeploymentError::InvalidAction);
        }
        plan.validate_action(effect.action)?;
        if intent.id() != original.deployment_id
            || intent.project_id() != target.prepared().project_id()
            || plan.ownership() != ResourceOwnership::Owned
            || plan.identity() != original.identity
            || plan.input_hash() != original.input_hash
            || effect.deployment_id != original.deployment_id
            || effect.resource != original.resource
            || effect.identity != original.identity
            || effect.input_hash != original.input_hash
            || configuration.namespace() != target.provider_namespace()
            || configuration.host() != target.host_id()
            || configuration.platform() != target.prepared().source().platform()
            || configuration.profile() != DeploymentExecutionProfile::RuntimeNamedV1
            || current_command.operation() != DeploymentOperation::Remove
            || effect.command.operation() != DeploymentOperation::Remove
            || effect.provenance.actor_id != effect.command.actor_id()
            || effect.provenance.request_id.as_uuid().is_nil()
            || effect.provenance.attempt_id == original.provenance.attempt_id
            || ![original, &effect].into_iter().all(|claim| {
                claim.generation > 0
                    && claim.generation <= i64::MAX.cast_unsigned()
                    && claim.resource_version > 0
                    && claim.resource_version <= i64::MAX.cast_unsigned()
            })
        {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(Self {
            intent,
            target,
            effect,
            current_command,
            configuration,
        })
    }
    /// Returns the full immutable intent for independent native comparison.
    #[must_use]
    pub const fn intent(&self) -> &DeploymentIntent {
        &self.intent
    }
    /// Returns the actual original Create/import comparison target.
    #[must_use]
    pub const fn target(&self) -> &PreparedInstanceSafetyTarget {
        &self.target
    }
    /// Returns the unchanged Detach-or-Delete claim, never current caller credentials.
    #[must_use]
    pub const fn effect(&self) -> &EffectClaim {
        &self.effect
    }
    /// Returns the actual current caller's logical Remove command.
    #[must_use]
    pub const fn current_command(&self) -> CommandIdentity {
        self.current_command
    }
    /// Returns complete neutral configuration pins, not ownership authority.
    #[must_use]
    pub const fn configuration(&self) -> &RemoveResumeConfiguration {
        &self.configuration
    }
}

/// Optional current-authorized initial terminal preparation boundary.
///
/// Implementations read full genuine closure and original CAS in one actor RRRO
/// snapshot with current ALL-owned rights. Returned strict requests are comparison
/// data; protected worker observation and owning completion remain mandatory.
#[async_trait]
pub trait PreparedInstanceTerminalPreparationRepository: Send + Sync + 'static {
    /// Reads the actual closure for the exact original Detach claim.
    ///
    /// # Errors
    /// Denies unsupported adapters, wrong action/role/configuration, lost current
    /// rights and absent, foreign or inconsistent original history.
    async fn prepare_detach(
        &self,
        _identity: &AuthenticatedIdentity,
        _input: &PreparedInstanceTerminalPreparationInput,
    ) -> Result<PreparedInstanceDetachRequest, DeploymentError> {
        Err(DeploymentError::InvalidAction)
    }
    /// Reads the actual closure and native original before-CAS for Delete.
    ///
    /// A missing transport comparison CAS must never become a nullable or guessed
    /// checked request. Actual committed history retains its saved original CAS.
    ///
    /// # Errors
    /// Denies unsupported adapters, lost rights, changed originals or unavailable,
    /// ambiguous, nonpositive or unrepresentable authoritative CAS.
    async fn prepare_delete(
        &self,
        _identity: &AuthenticatedIdentity,
        _input: &PreparedInstanceTerminalPreparationInput,
    ) -> Result<PreparedInstanceDeleteRequest, DeploymentError> {
        Err(DeploymentError::InvalidAction)
    }
}
