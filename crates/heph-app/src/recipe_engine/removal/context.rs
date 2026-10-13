//! One concrete tuple derived only from the original owning controller.
use super::RecipeRemovalError;
use crate::recipe::ready_retention::ReadyRetentionCoordinator;
use crate::{
    ManagedInvocationController, PreparedInstanceDrainCoordinator,
    PreparedInstanceTerminalCoordinator,
};
use recipe_application::{
    DeploymentExecutionProfile, PlatformPolicyObservation, RemoveResumeConfiguration,
};
use recipe_postgres::{
    PostgresDeploymentRepository, PostgresExecutionPreparationRepository,
    PostgresOwnedReadyRetentionRepository,
};
use std::sync::Arc;
use volume_local::{LocalVolumeStore, OwnedVolumeMetadata};
use volume_postgres::{
    PostgresReadyRetentionRepository, PostgresVolumeMetadataRepository, ScalarMetadataSchema,
};

pub(super) struct Context {
    pub controller: Arc<ManagedInvocationController>,
    pub deployment: PostgresDeploymentRepository,
    pub preparation: PostgresExecutionPreparationRepository,
    pub release: release_postgres::ReleaseService,
    pub drain: PreparedInstanceDrainCoordinator,
    pub terminal: PreparedInstanceTerminalCoordinator,
    pub retention: ReadyRetentionCoordinator,
    pub retention_actor: Arc<PostgresOwnedReadyRetentionRepository>,
    pub volumes: Arc<PostgresVolumeMetadataRepository>,
}
impl Context {
    pub fn new(controller: Arc<ManagedInvocationController>) -> Result<Self, RecipeRemovalError> {
        controller.validate_owners()?;
        let inputs = controller.drain_inputs();
        let configuration = &inputs.configuration;
        let scope = configuration.provider_scope();
        let platform = PlatformPolicyObservation::new(
            configuration.platform_policy().clone(),
            configuration.platform_version().to_owned(),
        )?;
        let remove = RemoveResumeConfiguration::new(
            scope.namespace().to_owned(),
            scope.host_id().to_owned(),
            platform.clone(),
            DeploymentExecutionProfile::RuntimeNamedV1,
        )?;
        let deployment = PostgresDeploymentRepository::new(inputs.pools.application.clone())
            .with_remove_resume(remove.clone());
        let preparation = PostgresExecutionPreparationRepository::new(
            inputs.pools.application.clone(),
            platform,
            DeploymentExecutionProfile::RuntimeNamedV1,
        )
        .with_remove_resume(remove)?;
        let actor_volumes = Arc::new(
            PostgresVolumeMetadataRepository::new(inputs.pools.application.clone())
                .with_scalar_schema(ScalarMetadataSchema::Profiled)
                .with_qualified_invocation_configuration(configuration.clone()),
        );
        let volumes = Arc::new(
            PostgresVolumeMetadataRepository::new(inputs.pools.worker.clone())
                .with_scalar_schema(ScalarMetadataSchema::Profiled)
                .with_qualified_invocation_configuration(configuration.clone()),
        );
        let local = Arc::new(
            LocalVolumeStore::new(volumes.clone(), inputs.local_volume.clone())?
                .with_owned_metadata(OwnedVolumeMetadata::new(
                    actor_volumes,
                    volumes.clone(),
                    inputs.volume_owner.clone(),
                ))?,
        );
        let retention_actor = Arc::new(PostgresOwnedReadyRetentionRepository::new(
            inputs.pools.application.clone(),
            preparation.clone(),
            scope.namespace().to_owned(),
            scope.host_id().to_owned(),
        )?);
        let worker = Arc::new(PostgresReadyRetentionRepository::new(
            inputs.pools.worker.clone(),
            scope.namespace().to_owned(),
            scope.host_id().to_owned(),
        )?);
        let provider: Arc<dyn heph_runtime::VmProvider> = inputs.provider.clone();
        let retention =
            ReadyRetentionCoordinator::new(retention_actor.clone(), worker, local, provider);
        let release = release_postgres::ReleaseService::new(
            inputs.pools.application.clone(),
            Arc::new(authz_postgres::PostgresMelangeAuthorizer),
        );
        let drain = PreparedInstanceDrainCoordinator::new(controller.clone())
            .map_err(|error| RecipeRemovalError::Drain(Arc::new(error)))?;
        let terminal = PreparedInstanceTerminalCoordinator::new(controller.clone())
            .map_err(|error| RecipeRemovalError::Terminal(Arc::new(error)))?;
        Ok(Self {
            controller,
            deployment,
            preparation,
            release,
            drain,
            terminal,
            retention,
            retention_actor,
            volumes,
        })
    }
}
