use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentExecutionPreparation, DeploymentExecutionPreparationRepository,
    DeploymentIntent, PreparedInstanceImport,
};
use release_domain::{AgentInstanceId, AgentInstanceRevisionId, ReleaseCommandKey};
use release_service::{ActivateInstance, InstanceActivationAdmission, InstanceActivationId};
use uuid::Uuid;

use super::{PreparedRecipeApplication, RecipeApplicationError, validation};

impl PreparedRecipeApplication {
    pub(super) async fn activate(
        &self,
        identity: &AuthenticatedIdentity,
        original: CommandIdentity,
        intent: &DeploymentIntent,
        frozen: &DeploymentExecutionPreparation,
    ) -> Vec<super::RecipeActivationAttempt> {
        let mut attempts = Vec::with_capacity(frozen.instances().len());
        for child in frozen.instances() {
            let result = self
                .activate_child(identity, original, intent, frozen, child)
                .await;
            attempts.push(super::RecipeActivationAttempt {
                resource: child.resource().clone(),
                result,
            });
        }
        attempts
    }

    async fn activate_child(
        &self,
        identity: &AuthenticatedIdentity,
        original: CommandIdentity,
        intent: &DeploymentIntent,
        frozen: &DeploymentExecutionPreparation,
        child: &PreparedInstanceImport,
    ) -> Result<InstanceActivationAdmission, RecipeApplicationError> {
        // Repeat live whole-input/source checks after each actual resource commit.
        let current = self
            .preparation
            .prepare_execution(identity, original, intent.id())
            .await?
            .preparation;
        validation::preparation(intent, original, &current)?;
        if &current != frozen {
            return Err(recipe_application::DeploymentError::InputConflict.into());
        }
        let snapshot = self.repository.inspect(identity, intent.id()).await?;
        validation::installing(&snapshot, intent)?;
        validation::ready(
            snapshot
                .resources
                .get(child.resource())
                .ok_or(recipe_application::DeploymentError::IntentMismatch)?,
        )?;
        let command = command(original, child)?;
        let admitted = self.activation.activate_instance(identity, command).await?;
        let revision_mismatch = admitted.revision_id() != command.expected_revision_id();
        if admitted.activation_id() != command.activation_id()
            || admitted.instance_id() != command.instance_id()
            || revision_mismatch
        {
            return Err(recipe_application::DeploymentError::IntentMismatch.into());
        }
        Ok(admitted)
    }
}

fn command(
    original: CommandIdentity,
    child: &PreparedInstanceImport,
) -> Result<ActivateInstance, release_service::InstanceExecutionError> {
    let mut name = b"hephaestus-prepared-recipe-activation-v1\0".to_vec();
    name.extend_from_slice(child.operation_id().as_bytes());
    let id = Uuid::new_v5(&original.id().as_uuid(), &name);
    ActivateInstance::new(
        ReleaseCommandKey::derive(
            "prepared_recipe_activation_v1",
            &[
                original.id().as_uuid().as_bytes(),
                child.operation_id().as_bytes(),
            ],
        ),
        InstanceActivationId::from_uuid(id)?,
        AgentInstanceId::from_uuid(child.instance_id().as_uuid()),
        AgentInstanceRevisionId::from_uuid(child.revision_id().as_uuid()),
        1,
    )
}
