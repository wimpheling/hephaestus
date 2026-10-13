//! Concrete owning preparation and existing provider cleanup; no authority wrapper.
use super::status::ManagedHeldReason;
use super::{ManagedServiceFailure, registry::Registry};
use crate::managed_backend::{acquire, custody, history, validation};
use crate::{AppConfig, AppError, ManagedInvocationPools};
use std::{error::Error, fmt, sync::Arc, time::Duration};
use tokio::sync::oneshot;

pub(super) struct PreparationInputs {
    pub(super) app: AppConfig,
    pub(super) pools: ManagedInvocationPools,
}

pub(super) struct Preparation {
    pub(super) inputs: Arc<PreparationInputs>,
    pub(super) result: oneshot::Sender<Result<(), ManagedServiceFailure>>,
}

pub(super) async fn prepare(registry: Arc<Registry>, request: Preparation) {
    let result = Box::pin(prepare_owned(
        &registry,
        &request.inputs.app,
        &request.inputs.pools,
    ))
    .await;
    let outcome = match result {
        Ok(()) => {
            registry.prepared();
            Ok(())
        }
        Err(PreparationError::Acquisition(cause)) => Err(registry.failed_preparation(cause, true)),
        Err(PreparationError::BeforeAcquisition(cause) | PreparationError::Construction(cause)) => {
            Err(registry.failed_preparation(cause, false))
        }
    };
    // Result delivery owns no originals. A cancelled ticket already requested close.
    drop(request.result.send(outcome));
}

enum PreparationError {
    BeforeAcquisition(AppError),
    Acquisition(acquire::AcquisitionFailure),
    Construction(AppError),
}

async fn prepare_owned(
    registry: &Arc<Registry>,
    app: &AppConfig,
    pools: &ManagedInvocationPools,
) -> Result<(), PreparationError> {
    let config = app.managed_backend.as_ref().ok_or_else(|| {
        PreparationError::BeforeAcquisition(AppError::Configuration(
            "managed service registration has no managed configuration".into(),
        ))
    })?;
    let checked = async {
        validation::validate(app, config)?;
        control_plane_postgres::verify_qualified_invocation_contract(
            &pools.admin,
            &pools.worker,
            &pools.application,
        )
        .await
        .map_err(custody::failure)?;
        validation::repositories(app, config, &pools.worker)?;
        history::observe(config, &pools.worker).await?;
        validation::validate(app, config)
    }
    .await
    .map_err(PreparationError::BeforeAcquisition);
    checked?;
    if registry.control.is_closing() {
        return Err(PreparationError::BeforeAcquisition(AppError::Task(
            "managed preparation abandoned before acquisition".into(),
        )));
    }
    let (provider, volume_owner) =
        acquire::owners(config).map_err(PreparationError::Acquisition)?;
    let inputs = custody::inputs(
        app,
        config,
        provider,
        volume_owner,
        &pools.admin,
        pools.worker.clone(),
        &pools.application,
    );
    let node = custody::node(&inputs, Arc::downgrade(registry));
    registry.install(Arc::clone(&node));
    if registry.control.is_closing() {
        return Err(PreparationError::Construction(AppError::Task(
            "managed preparation abandoned after acquisition".into(),
        )));
    }
    custody::construct(&node, inputs)
        .await
        .map_err(PreparationError::Construction)
}

pub(super) async fn drain(
    registry: Arc<Registry>,
    node: Arc<custody::ManagedBackendNode>,
    attempts: usize,
) {
    for index in 0..attempts {
        if index != 0 {
            tokio::time::sleep(if index == 1 {
                Duration::from_millis(250)
            } else {
                Duration::from_secs(1)
            })
            .await;
        }
        registry.attempt();
        custody::close_intake(&node).unwrap_or_else(|cause| {
            registry.record(cause, ManagedHeldReason::Services);
        });
        if let Err(cause) = node.services.start_cleanup() {
            registry.record(cause, ManagedHeldReason::Services);
        }
        if let Err(cause) = physical_stop(&node).await {
            tracing::warn!(error = %cause, attempt = index + 1, "managed same-owner physical cleanup remains held");
            registry.record(cause, ManagedHeldReason::PhysicalCleanup);
            continue;
        }
        let (services, recipes) =
            tokio::join!(node.services.join_mutations(), custody::join_recipes(&node));
        let failures = services
            .err()
            .into_iter()
            .chain(recipes.err())
            .collect::<Vec<_>>();
        if !failures.is_empty() {
            registry.record(
                custody::failure(CleanupFailures(failures)),
                ManagedHeldReason::Services,
            );
            break;
        }
        match custody::stop_runs(&node).await {
            Ok(Some(())) => {}
            Ok(None) => {
                registry.record(
                    AppError::Readiness(
                        "unassembled original node cannot qualify final Run reconciliation".into(),
                    ),
                    ManagedHeldReason::ReconciliationUnavailable,
                );
                break;
            }
            Err(cause) => {
                registry.record(cause, ManagedHeldReason::ReconciliationUnavailable);
                break;
            }
        }
        match node.services.close_owned().await {
            Ok(()) => {
                registry.stopped();
                return;
            }
            Err(cause) => {
                registry.record(cause, ManagedHeldReason::OrdinaryServicesCleanupUnavailable);
                break;
            }
        }
    }
    registry.drained();
}

async fn physical_stop(node: &custody::ManagedBackendNode) -> Result<(), AppError> {
    // Drain only actual registered generation custody before potentially stalled
    // startup/recipe/Core DB waits. Complete family reconciliation follows joins.
    let scope = node.configuration.provider_scope();
    let mut failures = Vec::new();
    if let Err(cause) = node
        .provider
        .verify_managed_configuration(&node.expected_provider, scope)
    {
        failures.push(custody::failure(cause));
    }
    if let Err(cause) = node.volume_owner.validate() {
        failures.push(custody::failure(cause));
    }
    // Closing validates its own exact retained scope/root. A volume failure must
    // not prevent this genuine VM admission close. No drain precedes that close.
    match node.provider.close_admission(scope).await {
        Ok(()) => {
            if let Err(cause) = node.provider.drain_generation(scope).await {
                failures.push(custody::failure(cause));
            }
        }
        Err(cause) => failures.push(custody::failure(cause)),
    }
    if let Err(cause) = node.volume_owner.validate() {
        failures.push(custody::failure(cause));
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(custody::failure(CleanupFailures(failures)))
    }
}

/// Every original outcome remains owned, including errors after an earlier failure.
#[derive(Debug)]
struct CleanupFailures(Vec<AppError>);
impl fmt::Display for CleanupFailures {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "managed same-owner closing retained {} failure(s)",
            self.0.len()
        )?;
        for cause in &self.0 {
            write!(f, "; {cause}")?;
        }
        Ok(())
    }
}
impl Error for CleanupFailures {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.0.first().map(|cause| cause as &(dyn Error + 'static))
    }
}
