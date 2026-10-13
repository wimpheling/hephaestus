//! Concrete production adapter assembly; caller has already completed all gates.

use super::{AppError, ManagedInvocationInputs, PgRunRepository, RunOrchestrator, component};
use crate::{CanonicalRunLaunchAuthorizer, PgAgentVmSpecFactory, PgRunAuthorityManager};
use control_plane_postgres::launch::PgRunLaunchAuthorizer;
use heph_runtime::VmProvider;
use run_domain::{FreshRunExecutionProfile, RunCleanupHostId};
use run_postgres::PgRunCleanupRepository;
use run_runtime_local::LocalRunRuntimeManager;
use runtime_authority_postgres::PgRuntimeSessionRepository;
use secret_postgres::{
    SecretRuntimeService, SecretService, initialize_qualified_invocation_manager,
};
use secret_runtime::FilesystemSecretMountProvider;
use secret_store::EncryptedStore;
use std::sync::Arc;
use volume_local::{LocalVolumeStore, OwnedVolumeMetadata};
use volume_postgres::{PostgresVolumeMetadataRepository, ScalarMetadataSchema};

pub(super) fn build(
    inputs: &ManagedInvocationInputs,
) -> Result<(Arc<PgRunRepository>, Arc<RunOrchestrator>), AppError> {
    let scope = RunCleanupHostId::new(
        inputs.configuration.provider_scope().namespace().to_owned(),
        inputs.configuration.provider_scope().host_id().to_owned(),
    )
    .map_err(component("qualified Invocation cleanup scope"))?;
    let runs = Arc::new(
        PgRunRepository::new(inputs.pools.worker.clone())
            .with_execution_profile(FreshRunExecutionProfile::OwnedCanonical)
            .with_vm_owner_scope(scope.clone())
            .with_qualified_invocation_configuration(inputs.configuration.clone())
            .with_qualified_invocation_lease_runtime()
            .with_qualified_cleanup_wake(scope.clone()),
    );
    let volumes = volumes(inputs)?;
    let cleanup = Arc::new(
        PgRunCleanupRepository::new(inputs.pools.worker.clone(), scope)
            .with_qualified_invocation_configuration(inputs.configuration.clone())
            .with_qualified_queued_invocation_cleanup(inputs.configuration.clone()),
    );
    let factory = Arc::new(
        PgAgentVmSpecFactory {
            pool: inputs.pools.worker.clone(),
            root_images: inputs.root_images.clone(),
            runtime_policy: inputs.runtime_policy.clone(),
        }
        .with_qualified_invocation(runs.clone(), runs.clone(), volumes.clone()),
    );
    // First filesystem initializer. Every validation and actual three-pool
    // preflight has completed before entering this assembly function.
    let runtime = Arc::new(
        LocalRunRuntimeManager::initialize_qualified_invocation(
            runs.clone(),
            inputs.run_runtime.clone(),
            runs.clone(),
            volumes.clone(),
        )
        .map_err(component("qualified Invocation runtime initialization"))?,
    );
    let authority = Arc::new(PgRunAuthorityManager::new_managed_invocation(
        inputs.pools.worker.clone(),
        (inputs.authority.root.clone(), inputs.authority.key),
        inputs.authority.session_ttl,
        inputs.configuration.clone(),
        runs.clone(),
        volumes.clone(),
    )?);
    let launch = Arc::new(CanonicalRunLaunchAuthorizer::new_qualified_invocation(
        PgRuntimeSessionRepository::new(inputs.pools.worker.clone()),
        Arc::new(PgRunLaunchAuthorizer::new(
            inputs.pools.worker.clone(),
            Arc::new(authz_postgres::PostgresMelangeAuthorizer),
        )),
        inputs.configuration.clone(),
        runs.clone(),
    ));
    let secrets = secrets(inputs)?;
    // Erase only this retained actual typed Arc, after its full configuration
    // and durable owner were verified. No provider is constructed or reopened.
    let provider: Arc<dyn VmProvider> = inputs.provider.clone();
    let orchestrator = Arc::new(
        RunOrchestrator::new(
            runs.clone(),
            volumes.clone(),
            provider,
            factory,
            inputs.instance_state_capacity_bytes,
        )
        .with_operation_guards(inputs.operation_guards.clone())
        .with_cleanup_repository(cleanup, volumes)
        .with_cleanup_timeout(inputs.cleanup_timeout)
        .with_volume_preparation()
        .with_qualified_invocation(inputs.configuration.provider_scope().clone())
        .with_runtime_manager(runtime)
        .with_launch_authorizer(launch)
        .with_authority_manager(authority)
        .with_secret_manager(secrets),
    );
    Ok((runs, orchestrator))
}

fn volumes(inputs: &ManagedInvocationInputs) -> Result<Arc<LocalVolumeStore>, AppError> {
    let worker = Arc::new(
        PostgresVolumeMetadataRepository::new(inputs.pools.worker.clone())
            .with_scalar_schema(ScalarMetadataSchema::Profiled)
            .with_qualified_invocation_configuration(inputs.configuration.clone()),
    );
    let actor = Arc::new(
        PostgresVolumeMetadataRepository::new(inputs.pools.application.clone())
            .with_scalar_schema(ScalarMetadataSchema::Profiled)
            .with_qualified_invocation_configuration(inputs.configuration.clone()),
    );
    let local = LocalVolumeStore::new(worker.clone(), inputs.local_volume.clone())
        .map_err(component("qualified Invocation local volume configuration"))?
        .with_run_metadata(worker.clone())
        .with_owned_metadata(OwnedVolumeMetadata::new(
            actor,
            worker,
            inputs.volume_owner.clone(),
        ))
        .map_err(component("qualified Invocation owned volume metadata"))?;
    Ok(Arc::new(local))
}

fn secrets(
    inputs: &ManagedInvocationInputs,
) -> Result<Arc<dyn heph_run::RunSecretManager>, AppError> {
    let authorizer = Arc::new(authz_postgres::PostgresMelangeAuthorizer);
    let dispatch = SecretService::new(
        inputs.pools.application.clone(),
        EncryptedStore::new(inputs.secret_keys.clone()),
        authorizer.clone(),
    );
    let runtime = SecretRuntimeService::new(
        inputs.pools.application.clone(),
        inputs.pools.worker.clone(),
        EncryptedStore::new(inputs.secret_keys.clone()),
        authorizer,
    )
    .with_mount_provider(Arc::new(FilesystemSecretMountProvider::new()));
    let manager = initialize_qualified_invocation_manager(
        inputs.pools.worker.clone(),
        dispatch,
        runtime,
        inputs.secret_mounts.clone(),
        inputs.configuration.clone(),
    )
    .map_err(component("qualified Invocation saved-empty secret manager"))?;
    Ok(Arc::new(manager))
}
