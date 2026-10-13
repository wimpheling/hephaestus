//! Concrete adapters derive only from the original controller's retained tuple.
use super::{Actions, AppError, Arc, ManagedInvocationController, Outcome, Request};
use crate::{
    recipe_engine::PreparedRecipeApplication, recipe_instance_import::PreparedInstanceImporter,
    recipe_owned_volume::PreparedOwnedVolumeCreator,
    recipe_retained_volume::PreparedRetainedVolumeVerifier,
};
use recipe_application::{
    DeploymentExecutionProfile, PlatformPolicyObservation, RecipePlanner, RemoveResumeConfiguration,
};
use recipe_postgres::{
    PostgresDeploymentRepository, PostgresEffectVerificationRecorder,
    PostgresExecutionPreparationRepository, PostgresOwnedReadyRetentionRepository,
    PostgresPlanningCatalog,
};
use volume_local::{LocalVolumeStore, OwnedVolumeMetadata};
use volume_postgres::{
    PostgresReadyRetentionRepository, PostgresVolumeMetadataRepository, ScalarMetadataSchema,
};

struct Original {
    controller: Arc<ManagedInvocationController>,
    engine: PreparedRecipeApplication,
}
#[async_trait::async_trait]
impl Actions for Original {
    async fn run(&self, request: &Request) -> Outcome {
        self.controller
            .validate_owners()
            .map_err(|cause| Arc::new(super::Failure::Owner(cause)))?;
        let outcome = match request {
            Request::Install {
                identity, request, ..
            } => self.engine.install(identity, request).await,
            Request::Resume {
                identity,
                original,
                deployment,
                expected_project,
            } => {
                super::resume::run(
                    &self.engine,
                    identity,
                    *original,
                    *deployment,
                    *expected_project,
                )
                .await
            }
        };
        outcome
            .map(Arc::new)
            .map_err(|cause| Arc::new(super::Failure::Execution(cause)))
    }
}
pub(super) fn assemble(
    controller: Arc<ManagedInvocationController>,
) -> Result<Arc<dyn Actions>, AppError> {
    controller.validate_owners()?;
    let cfg = &controller.drain_inputs().configuration;
    let scope = cfg.provider_scope();
    let platform = PlatformPolicyObservation::new(
        cfg.platform_policy().clone(),
        cfg.platform_version().to_owned(),
    )
    .map_err(crate::managed_backend::custody::failure)?;
    let remove = RemoveResumeConfiguration::new(
        scope.namespace().to_owned(),
        scope.host_id().to_owned(),
        platform.clone(),
        DeploymentExecutionProfile::RuntimeNamedV1,
    )
    .map_err(crate::managed_backend::custody::failure)?;
    let ledger = Arc::new(
        PostgresDeploymentRepository::new(controller.drain_inputs().pools.application.clone())
            .with_remove_resume(remove.clone()),
    );
    let preparation = Arc::new(
        PostgresExecutionPreparationRepository::new(
            controller.drain_inputs().pools.application.clone(),
            platform.clone(),
            DeploymentExecutionProfile::RuntimeNamedV1,
        )
        .with_remove_resume(remove)
        .map_err(crate::managed_backend::custody::failure)?,
    );
    let recorder = Arc::new(PostgresEffectVerificationRecorder::new(
        controller.drain_inputs().pools.worker.clone(),
    ));
    let (volumes, retained) = volumes(controller.drain_inputs(), &preparation, &ledger, &recorder)?;
    let authorizer = Arc::new(authz_postgres::PostgresMelangeAuthorizer);
    let release = Arc::new(release_postgres::ReleaseService::new(
        controller.drain_inputs().pools.application.clone(),
        authorizer.clone(),
    ));
    let importer = Arc::new(PreparedInstanceImporter::new(
        preparation.clone(),
        ledger.clone(),
        release,
        recorder,
    ));
    let activation = Arc::new(
        release_postgres::PostgresInstanceExecutionService::new(
            controller.drain_inputs().pools.application.clone(),
            authorizer,
            cfg.clone(),
        )
        .with_qualified_invocation_admission(),
    );
    let engine = PreparedRecipeApplication::new(
        Arc::new(RecipePlanner::new(PostgresPlanningCatalog::new(
            controller.drain_inputs().pools.application.clone(),
            platform,
        ))),
        ledger,
        preparation,
        volumes,
        importer,
        activation,
    )
    .with_retained_volume_verifier(retained);
    Ok(Arc::new(Original { controller, engine }))
}

fn volumes(
    inputs: &crate::ManagedInvocationInputs,
    preparation: &Arc<PostgresExecutionPreparationRepository>,
    ledger: &Arc<PostgresDeploymentRepository>,
    recorder: &Arc<PostgresEffectVerificationRecorder>,
) -> Result<
    (
        Arc<PreparedOwnedVolumeCreator>,
        Arc<PreparedRetainedVolumeVerifier>,
    ),
    AppError,
> {
    let cfg = &inputs.configuration;
    let scope = cfg.provider_scope();
    let actor = Arc::new(
        PostgresVolumeMetadataRepository::new(inputs.pools.application.clone())
            .with_scalar_schema(ScalarMetadataSchema::Profiled)
            .with_qualified_invocation_configuration(cfg.clone()),
    );
    let worker = Arc::new(
        PostgresVolumeMetadataRepository::new(inputs.pools.worker.clone())
            .with_scalar_schema(ScalarMetadataSchema::Profiled)
            .with_qualified_invocation_configuration(cfg.clone()),
    );
    let creator = PreparedOwnedVolumeCreator::new(
        preparation.clone(),
        ledger.clone(),
        actor.clone(),
        worker.clone(),
        recorder.clone(),
        inputs.local_volume.clone(),
        inputs.volume_owner.clone(),
    )
    .map_err(crate::managed_backend::custody::failure)?;
    let local = Arc::new(
        LocalVolumeStore::new(worker.clone(), inputs.local_volume.clone())
            .and_then(|local| {
                local.with_owned_metadata(OwnedVolumeMetadata::new(
                    actor,
                    worker.clone(),
                    inputs.volume_owner.clone(),
                ))
            })
            .map_err(crate::managed_backend::custody::failure)?,
    );
    let retention_actor = Arc::new(
        PostgresOwnedReadyRetentionRepository::new(
            inputs.pools.application.clone(),
            preparation.as_ref().clone(),
            scope.namespace().to_owned(),
            scope.host_id().to_owned(),
        )
        .map_err(crate::managed_backend::custody::failure)?,
    );
    let retention_worker = Arc::new(
        PostgresReadyRetentionRepository::new(
            inputs.pools.worker.clone(),
            scope.namespace().to_owned(),
            scope.host_id().to_owned(),
        )
        .map_err(crate::managed_backend::custody::failure)?,
    );
    let provider: Arc<dyn heph_runtime::VmProvider> = inputs.provider.clone();
    let retained = PreparedRetainedVolumeVerifier::new(
        retention_actor,
        retention_worker,
        worker,
        recorder.clone(),
        ledger.clone(),
        local,
        inputs.volume_owner.clone(),
        provider,
    )
    .map_err(crate::managed_backend::custody::failure)?;
    Ok((Arc::new(creator), Arc::new(retained)))
}
