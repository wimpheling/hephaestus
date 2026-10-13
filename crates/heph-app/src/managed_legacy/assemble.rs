//! Ordinary production composition after both read-only gates and PG preflight.

use super::{
    AppError, LegacyVmPlacementScope, ManagedLegacyInputs, PgRunRepository, RunOrchestrator,
    component, producers, secrets, validation,
};
use crate::{PgAgentVmSpecFactory, PgRunAuthorityManager};
use control_plane_postgres::launch::PgRunLaunchAuthorizer;
use heph_runtime::VmProvider;
use mailbox_dispatch::MailboxRunResources;
use run_domain::FreshRunExecutionProfile;
use run_runtime_local::LocalRunRuntimeManager;
use std::sync::Arc;
use volume_local::LocalVolumeStore;
use volume_postgres::{PostgresVolumeMetadataRepository, ScalarMetadataSchema};
use workspace_local::LocalWorkspaceManager;
use workspace_postgres::PgWorkspaceMetadataRepository;

pub(super) struct Composition {
    pub(super) scope: LegacyVmPlacementScope,
    pub(super) runs: Arc<PgRunRepository>,
    pub(super) orchestrator: Arc<RunOrchestrator>,
    pub(super) producers: producers::Producers,
}

pub(super) async fn build(inputs: &ManagedLegacyInputs) -> Result<Composition, AppError> {
    let scope = LegacyVmPlacementScope::new(inputs.configuration.provider_scope().clone())
        .map_err(component("managed Legacy placement scope"))?;
    let runs = Arc::new(
        PgRunRepository::new(inputs.pools.worker.clone())
            .with_execution_profile(FreshRunExecutionProfile::LegacyScalar)
            .with_legacy_vm_placement(scope.clone()),
    );
    let metadata = Arc::new(
        PostgresVolumeMetadataRepository::new(inputs.pools.worker.clone())
            .with_scalar_schema(ScalarMetadataSchema::Profiled),
    );
    let volumes = Arc::new(
        LocalVolumeStore::new(metadata, inputs.local_volume.clone())
            .map_err(component("managed Legacy scalar volume configuration"))?,
    );
    let workspace_repository = Arc::new(PgWorkspaceMetadataRepository::new(
        inputs.pools.worker.clone(),
    ));
    let mut workspaces = LocalWorkspaceManager::new(
        workspace_repository.clone(),
        workspace_repository,
        inputs.workspaces.clone(),
    )
    .map_err(component("managed Legacy workspace configuration"))?;
    let factory = Arc::new(PgAgentVmSpecFactory {
        pool: inputs.pools.worker.clone(),
        root_images: inputs.root_images.clone(),
        runtime_policy: inputs.runtime_policy.clone(),
    });
    let producers = producers::build(inputs, &scope);
    let secret_manager = secrets::build(inputs)?;
    // First filesystem initializer. Actual three-pool preflight and repeated
    // provider/owner/config/root/socket checks completed before entering here.
    validation::validate(inputs)?;
    volumes
        .initialize()
        .await
        .map_err(component("managed Legacy volume initialization"))?;
    validation::validate(inputs)?;
    workspaces
        .initialize()
        .map_err(component("managed Legacy workspace initialization"))?;
    let workspaces = Arc::new(workspaces);
    validation::validate(inputs)?;
    let runtime = Arc::new(
        LocalRunRuntimeManager::initialize(runs.clone(), inputs.run_runtime.clone())
            .map_err(component("managed Legacy runtime initialization"))?,
    );
    validation::validate(inputs)?;
    let authority = Arc::new(PgRunAuthorityManager::new_qualified_legacy(
        inputs.pools.worker.clone(),
        (inputs.authority.root.clone(), inputs.authority.key),
        inputs.authority.session_ttl,
        scope.clone(),
    )?);
    let launch = Arc::new(PgRunLaunchAuthorizer::new(
        inputs.pools.worker.clone(),
        Arc::new(authz_postgres::PostgresMelangeAuthorizer),
    ));
    // Erase only the SAME retained, fully checked concrete provider Arc.
    let provider: Arc<dyn VmProvider> = inputs.provider.clone();
    let orchestrator = Arc::new(
        RunOrchestrator::new(
            runs.clone(),
            volumes,
            provider,
            factory,
            inputs.instance_state_capacity_bytes,
        )
        .with_operation_guards(inputs.operation_guards.clone())
        .with_legacy_vm_placement(scope.clone())
        .with_cleanup_timeout(inputs.cleanup_timeout)
        .with_workspace_manager(workspaces.clone())
        .with_runtime_git_workspace_manager(workspaces)
        .with_runtime_manager(runtime)
        .with_launch_authorizer(launch)
        .with_resource_observer(Arc::new(MailboxRunResources::new(
            producers.mailbox_store.clone(),
        )))
        .with_authority_manager(authority)
        .with_secret_manager(secret_manager)
        .with_completion_observer(producers.completion.clone()),
    );
    Ok(Composition {
        scope,
        runs,
        orchestrator,
        producers,
    })
}
