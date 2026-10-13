//! Exact external claim admission; retained qualification belongs to the owning port.

use capability_domain::CapabilitySlotKey;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentError,
    DeploymentExecutionPreparation, DeploymentExecutionPreparationRepository, DeploymentIntent,
    DeploymentOperation, EffectClaim, InstallProgress, PlannedResourceIdentity, RemovalProgress,
    ResourceAction, ResourceEffect, ResourceOwnership, ResourceProgress,
};
use uuid::Uuid;

use super::{PreparedRecipeApplication, RecipeApplicationError, validation};

impl PreparedRecipeApplication {
    pub(super) async fn verify_external(
        &self,
        identity: &AuthenticatedIdentity,
        command: CommandIdentity,
        intent: &DeploymentIntent,
        frozen: &DeploymentExecutionPreparation,
        resource: &CapabilitySlotKey,
    ) -> Result<ResourceProgress, RecipeApplicationError> {
        let verifier = self
            .retained_volumes
            .as_ref()
            .ok_or_else(|| RecipeApplicationError::UnsupportedExternal(resource.clone()))?;
        command.validate(identity, DeploymentOperation::Install)?;
        let target = intent
            .resources()
            .get(resource)
            .ok_or(DeploymentError::IntentMismatch)?;
        if target.ownership() != ResourceOwnership::External
            || !matches!(target.identity(), PlannedResourceIdentity::Volume { .. })
        {
            return Err(RecipeApplicationError::UnsupportedExternal(
                resource.clone(),
            ));
        }
        let current = self
            .preparation
            .prepare_execution(identity, command, intent.id())
            .await?
            .preparation;
        validation::preparation(intent, command, &current)?;
        if &current != frozen {
            return Err(DeploymentError::InputConflict.into());
        }
        let claim = if let Some(claim) = self
            .preparation
            .load_retained_external_verification_claim(identity, command, intent.id(), resource)
            .await?
        {
            claim // Active or completed genuine original claim; Ready itself grants nothing.
        } else {
            self.claim_external(identity, command, intent, resource)
                .await?
        };
        if claim.command != command
            || claim.deployment_id != intent.id()
            || &claim.resource != resource
            || claim.action != ResourceAction::VerifyExternal
            || claim.identity != target.identity()
            || claim.input_hash != target.input_hash()
            || claim.provenance.actor_id != command.actor_id()
        {
            return Err(DeploymentError::InputConflict.into());
        }
        // No ordinary-external fallback, standalone reopening or generic Ready
        // shortcut follows this actual retained actor/worker/Local boundary.
        verifier
            .verify_retained_volume(identity, &claim)
            .await
            .map_err(|error| RecipeApplicationError::RetainedVolume(Box::new(error)))
    }

    async fn claim_external(
        &self,
        identity: &AuthenticatedIdentity,
        command: CommandIdentity,
        intent: &DeploymentIntent,
        resource: &CapabilitySlotKey,
    ) -> Result<EffectClaim, DeploymentError> {
        // None was qualified only for pristine generation zero. A concurrent
        // effect cannot turn its old absence into a replacement verification.
        let snapshot = self.repository.inspect(identity, intent.id()).await?;
        validation::installing(&snapshot, intent)?;
        let progress = snapshot
            .resources
            .get(resource)
            .ok_or(DeploymentError::IntentMismatch)?;
        if progress.install != InstallProgress::Pending
            || progress.removal != RemovalProgress::Retained
            || progress.version != 0
            || progress.active_attempt.is_some()
            || progress.diagnostic.is_some()
        {
            return Err(DeploymentError::ReconciliationRequired);
        }
        self.repository
            .claim_resource_effect(
                identity,
                ResourceEffect {
                    command,
                    deployment_id: intent.id(),
                    resource: resource.clone(),
                    expected_deployment_version: snapshot.version,
                    expected_resource_version: progress.version,
                    action: ResourceAction::VerifyExternal,
                    provenance: AttemptProvenance::new(
                        identity,
                        DeploymentAttemptId::from_uuid(Uuid::new_v4())?,
                    )?,
                },
            )
            .await
    }
}
