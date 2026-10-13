//! Private original-node custody across awaited construction and caller cancellation.

use super::{AppConfig, AppError, ManagedBackendConfig};
use crate::{
    ManagedInvocationAuthority, ManagedInvocationInputs, ManagedInvocationPools,
    ManagedOrdinaryAdmissionPolicy, ManagedRunCommandExecutor, ManagedRunCommandInputs,
};
use control_plane_postgres::ControlPlanePool;
use release_service::InstanceExecutionConfiguration;
use run_orchestrator::RunOperationGuards;
use std::{
    error::Error,
    sync::{Arc, OnceLock},
};
use vm_libkrun::{LibkrunConfig, LibkrunProvider};
use volume_local::VolumeRootOwner;

/// Original tuple retained by the process-owned service, never by a result waiter.
pub(super) struct ManagedBackendNode {
    pub(super) provider: Arc<LibkrunProvider>,
    pub(super) expected_provider: LibkrunConfig,
    pub(super) volume_owner: Arc<VolumeRootOwner>,
    _guards: Arc<RunOperationGuards>,
    pub(super) configuration: InstanceExecutionConfiguration,
    _pools: ManagedInvocationPools,
    executor: OnceLock<Arc<ManagedRunCommandExecutor>>,
    pub(super) services: Arc<super::services::Services>,
}

#[cfg(test)]
pub(super) fn pending_readiness(assembled: bool) -> Result<(), AppError> {
    Err(AppError::Readiness(
        if assembled {
            "managed backend is pending recovery, command dispatch and shared shutdown custody"
        } else {
            "managed backend construction is still pending"
        }
        .to_owned(),
    ))
}

pub(super) fn inputs(
    app: &AppConfig,
    config: &ManagedBackendConfig,
    provider: Arc<LibkrunProvider>,
    volume_owner: Arc<VolumeRootOwner>,
    admin: &ControlPlanePool,
    worker: ControlPlanePool,
    application: &ControlPlanePool,
) -> ManagedRunCommandInputs {
    ManagedRunCommandInputs {
        shared: ManagedInvocationInputs {
            provider,
            expected_provider: config.provider.clone(),
            volume_owner,
            configuration: config.configuration.clone(),
            pools: ManagedInvocationPools {
                admin: admin.clone(),
                worker,
                application: application.clone(),
            },
            local_volume: config.volumes.clone(),
            run_runtime: config.run_runtime.clone(),
            root_images: app.root_images.clone(),
            runtime_policy: app.runtime_policy.clone(),
            secret_keys: app.secret_keys.clone(),
            secret_mounts: config.secret_mounts.clone(),
            authority: ManagedInvocationAuthority {
                root: config.authority_root.clone(),
                key: config.authority_key,
                session_ttl: config.authority_session_ttl,
            },
            instance_state_capacity_bytes: app.agent_state_capacity_bytes,
            cleanup_timeout: config.cleanup_timeout,
            operation_guards: Arc::new(RunOperationGuards::default()),
        },
        workspaces: config.workspaces.clone(),
        runtime_git_socket: config.runtime_git_socket.clone(),
        secret_broker_socket: app.secret_broker_socket.clone(),
        ordinary_policy: ManagedOrdinaryAdmissionPolicy::LegacyCompatibility,
    }
}

pub(super) fn node(inputs: &ManagedRunCommandInputs) -> Arc<ManagedBackendNode> {
    let shared = &inputs.shared;
    Arc::new(ManagedBackendNode {
        provider: shared.provider.clone(),
        expected_provider: shared.expected_provider.clone(),
        volume_owner: shared.volume_owner.clone(),
        _guards: shared.operation_guards.clone(),
        configuration: shared.configuration.clone(),
        _pools: ManagedInvocationPools {
            admin: shared.pools.admin.clone(),
            worker: shared.pools.worker.clone(),
            application: shared.pools.application.clone(),
        },
        executor: OnceLock::new(),
        services: Arc::new(super::services::Services::default()),
    })
}

pub(super) async fn construct(
    node: &ManagedBackendNode,
    inputs: ManagedRunCommandInputs,
) -> Result<(), AppError> {
    // Registry custody is installed before this first actual controller await.
    let executor = Arc::new(Box::pin(ManagedRunCommandExecutor::new(inputs)).await?);
    node.executor
        .set(Arc::clone(&executor))
        .map_err(|_| AppError::Task("managed executor already assembled".into()))?;
    // Store the SAME executor before owned recovery's first await. Successful
    // recovery still leaves public listener readiness at PendingRecovery.
    executor
        .reconcile_lifecycle(run_orchestrator::SupervisorReconciliationCause::Restart)
        .await
        .map_err(failure)
}

pub(super) fn executor(
    node: &ManagedBackendNode,
) -> Result<Arc<ManagedRunCommandExecutor>, AppError> {
    node.executor
        .get()
        .cloned()
        .ok_or_else(|| AppError::Readiness("managed original executor is not assembled".into()))
}

pub(super) fn close_intake(node: &ManagedBackendNode) -> Result<(), AppError> {
    if let Some(executor) = node.executor.get() {
        executor.close_intake().map_err(failure)?;
    }
    node.services.request_close();
    Ok(())
}

/// An assembled node owns qualified Stop; None is explicitly unassembled.
pub(super) async fn stop_runs(node: &ManagedBackendNode) -> Result<Option<()>, AppError> {
    let Some(executor) = node.executor.get() else {
        return Ok(None);
    };
    executor.close_intake().map_err(failure)?;
    executor
        .reconcile_lifecycle(run_orchestrator::SupervisorReconciliationCause::Stop)
        .await
        .map_err(failure)?;
    Ok(Some(()))
}

pub(super) fn failure(error: impl Error + Send + Sync + 'static) -> AppError {
    AppError::ManagedBackend {
        source: Box::new(error),
    }
}
