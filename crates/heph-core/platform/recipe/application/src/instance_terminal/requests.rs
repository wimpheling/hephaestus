use crate::{
    CommandIdentity, DeploymentError, DeploymentExecutionProfile, DeploymentIntent,
    DeploymentOperation, EffectClaim, OriginalInstanceClosure, PreparedInstanceSafetyTarget,
    RemoveResumeConfiguration, ResourceAction, ResourceOwnership,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Input {
    intent: DeploymentIntent,
    target: PreparedInstanceSafetyTarget,
    closure: OriginalInstanceClosure,
    effect: EffectClaim,
    current_command: CommandIdentity,
    configuration: RemoveResumeConfiguration,
}
impl Input {
    fn checked(self, action: ResourceAction) -> Result<Self, DeploymentError> {
        let original = self.target.original();
        let plan = self
            .intent
            .resources()
            .get(&original.resource)
            .ok_or(DeploymentError::IntentMismatch)?;
        self.closure.validate(original)?;
        self.closure.validate(&self.effect)?;
        plan.validate_action(action)?;
        let same_plan = self.effect.deployment_id == original.deployment_id
            && self.effect.resource == original.resource
            && self.effect.identity == original.identity
            && self.effect.input_hash == original.input_hash;
        let same_configuration = self.configuration.namespace() == self.target.provider_namespace()
            && self.configuration.host() == self.target.host_id()
            && self.configuration.platform() == self.target.prepared().source().platform()
            && self.configuration.profile() == DeploymentExecutionProfile::RuntimeNamedV1;
        if self.intent.id() != original.deployment_id
            || self.intent.project_id() != self.target.prepared().project_id()
            || plan.ownership() != ResourceOwnership::Owned
            || plan.identity() != original.identity
            || plan.input_hash() != original.input_hash
            || !same_plan
            || !same_configuration
            || self.current_command.operation() != DeploymentOperation::Remove
            || self.effect.command.operation() != DeploymentOperation::Remove
            || self.effect.action != action
            || self.effect.provenance.actor_id != self.effect.command.actor_id()
            || self.effect.provenance.request_id.as_uuid().is_nil()
            || ![original, &self.closure.drain, &self.effect]
                .into_iter()
                .all(|claim| {
                    positive_bigint(claim.generation) && positive_bigint(claim.resource_version)
                })
            || self.closure.drain.provenance.attempt_id == original.provenance.attempt_id
            || self.effect.provenance.attempt_id == original.provenance.attempt_id
            || self.effect.provenance.attempt_id == self.closure.drain.provenance.attempt_id
        {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(self)
    }
}
const fn positive_bigint(value: u64) -> bool {
    value > 0 && value <= i64::MAX.cast_unsigned()
}

/// Exact active Detach comparison input, separate from current publisher authority.
///
/// The native adapter must independently prove active status, admission, ALL-owned
/// rights, original associations and physical cleanup. Historical claims are data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInstanceDetachRequest(Input);
impl PreparedInstanceDetachRequest {
    /// Retains full originals and trusted configuration with a current Remove command.
    ///
    /// The current command may equal the original effect command or belong to a
    /// genuinely admitted fresh manager; this constructor certifies neither case.
    ///
    /// # Errors
    /// Rejects non-Detach/Remove shapes, substituted immutable plan/closure/scope,
    /// unsupported policy, reused attempt identities or invalid fencing provenance.
    pub fn new(
        intent: DeploymentIntent,
        target: PreparedInstanceSafetyTarget,
        closure: OriginalInstanceClosure,
        effect: EffectClaim,
        current_command: CommandIdentity,
        configuration: RemoveResumeConfiguration,
    ) -> Result<Self, DeploymentError> {
        Input {
            intent,
            target,
            closure,
            effect,
            current_command,
            configuration,
        }
        .checked(ResourceAction::Detach)
        .map(Self)
    }
    /// Returns the full immutable declaration and resolved graph.
    #[must_use]
    pub const fn intent(&self) -> &DeploymentIntent {
        &self.0.intent
    }
    /// Returns original Create/import/provider comparison pins, never credentials.
    #[must_use]
    pub const fn target(&self) -> &PreparedInstanceSafetyTarget {
        &self.0.target
    }
    /// Returns the unchanged original Drain and permanent closure.
    #[must_use]
    pub const fn closure(&self) -> &OriginalInstanceClosure {
        &self.0.closure
    }
    /// Returns the entire original active Detach claim for native requalification.
    #[must_use]
    pub const fn effect(&self) -> &EffectClaim {
        &self.0.effect
    }
    /// Returns only the current caller's logical Remove command to authenticate.
    #[must_use]
    pub const fn current_command(&self) -> CommandIdentity {
        self.0.current_command
    }
    /// Returns trusted composition comparison data, not ownership authority.
    #[must_use]
    pub const fn configuration(&self) -> &RemoveResumeConfiguration {
        &self.0.configuration
    }
}

/// Exact active Delete comparison input with explicit instance CAS.
///
/// No Detach observation is accepted as proof. The protected native adapter must
/// read actual committed Detach transitions and complete original/physical history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInstanceDeleteRequest {
    input: Input,
    expected_instance_version: u64,
}
impl PreparedInstanceDeleteRequest {
    /// Retains exact Delete inputs without authenticating a historical principal.
    ///
    /// # Errors
    /// Rejects changed originals/configuration, forbidden policy/action, invalid
    /// provenance or a zero/out-of-bigint expected instance version.
    pub fn new(
        intent: DeploymentIntent,
        target: PreparedInstanceSafetyTarget,
        closure: OriginalInstanceClosure,
        effect: EffectClaim,
        current_command: CommandIdentity,
        configuration: RemoveResumeConfiguration,
        expected_instance_version: u64,
    ) -> Result<Self, DeploymentError> {
        if !positive_bigint(expected_instance_version) {
            return Err(DeploymentError::StaleClaim);
        }
        let input = Input {
            intent,
            target,
            closure,
            effect,
            current_command,
            configuration,
        }
        .checked(ResourceAction::Delete)?;
        Ok(Self {
            input,
            expected_instance_version,
        })
    }
    /// Returns the full immutable declaration and resolved graph.
    #[must_use]
    pub const fn intent(&self) -> &DeploymentIntent {
        &self.input.intent
    }
    /// Returns the complete original Create/import/provider comparison input.
    #[must_use]
    pub const fn target(&self) -> &PreparedInstanceSafetyTarget {
        &self.input.target
    }
    /// Returns the original Drain and closure without replacing its provenance.
    #[must_use]
    pub const fn closure(&self) -> &OriginalInstanceClosure {
        &self.input.closure
    }
    /// Returns the original active Delete claim, not the fresh publisher identity.
    #[must_use]
    pub const fn effect(&self) -> &EffectClaim {
        &self.input.effect
    }
    /// Returns the current caller's Remove command for independent authorization.
    #[must_use]
    pub const fn current_command(&self) -> CommandIdentity {
        self.input.current_command
    }
    /// Returns exact trusted config to independently compare with native originals.
    #[must_use]
    pub const fn configuration(&self) -> &RemoveResumeConfiguration {
        &self.input.configuration
    }
    /// Returns explicit before-effect instance CAS, not a guessed activation version.
    #[must_use]
    pub const fn expected_instance_version(&self) -> u64 {
        self.expected_instance_version
    }
}
