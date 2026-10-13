//! Owning, explicitly configured prepared-instance Drain; never instance Delete.

mod admission;
mod custody;
mod execution;
#[cfg(test)]
mod tests;

use crate::{AppError, ManagedInvocationController};
use capability_domain::CapabilitySlotKey;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentError, DeploymentExecutionProfile, DeploymentId, PlatformPolicyObservation,
    RemoveDeployment, RemoveResumeConfiguration, ResourceProgress,
};
use recipe_postgres::{
    PostgresDeploymentRepository, PostgresEffectVerificationRecorder,
    PostgresExecutionPreparationRepository,
};
use release_postgres::PostgresPreparedInstanceSafetyRepository;
use run_postgres::PgInstanceDrainReader;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

/// A truthful held boundary preserves its original error and original owning task.
#[derive(Debug, thiserror::Error)]
pub enum PreparedInstanceDrainError {
    /// Current actor admission, exact history or worker proof failed.
    #[error(transparent)]
    Deployment(#[from] DeploymentError),
    /// Original physical/configuration custody changed.
    #[error(transparent)]
    Custody(#[from] AppError),
    /// Original controller rejected comparison or actual cancellation cleanup.
    #[error(transparent)]
    Controller(#[from] run_orchestrator::OrchestratorError),
    /// Native history has no actionable DTO or full Drained proof yet.
    #[error("prepared instance Drain remains pending; original owner retained")]
    Pending,
    /// The owning reply channel failed; never evidence of completed cleanup.
    #[error("prepared instance Drain outcome is unknown; original owner retained")]
    OutcomeUnknown,
}

/// Safe visibility of registered work; no provider handle or cleanup authority.
#[derive(Debug, Clone)]
pub struct PreparedInstanceDrainPending {
    /// Exact deployment with a registered resource owner.
    pub deployment_id: DeploymentId,
    /// Validated original resource name.
    pub resource: CapabilitySlotKey,
    /// Original Remove command, never a reconstructed historical identity.
    pub original_command: recipe_application::CommandIdentity,
    /// Retained original claim attempt, if its admission returned.
    pub attempt: Option<recipe_application::DeploymentAttemptId>,
    /// An error or pending observation requires a fresh current request.
    pub held: bool,
}

/// One concrete coordinator derived only from the original owning controller.
///
/// Registered tasks retain that SAME controller and all exact original progress
/// across waiter cancellation. Held owners accept fresh currently authenticated
/// retries; runtime destruction remains a process crash, not confirmed cleanup.
/// This opt-in service establishes no managed readiness, instance deletion or
/// `DeploymentRemoved`. It must remain reachable by its process composition owner.
pub struct PreparedInstanceDrainCoordinator {
    context: Arc<Context>,
    pending: Arc<Mutex<BTreeMap<custody::Key, custody::Control>>>,
}

struct Context {
    controller: Arc<ManagedInvocationController>,
    deployment: PostgresDeploymentRepository,
    preparation: PostgresExecutionPreparationRepository,
    safety: PostgresPreparedInstanceSafetyRepository,
    reader: PgInstanceDrainReader,
    recorder: PostgresEffectVerificationRecorder,
}

#[derive(Clone)]
struct Request {
    identity: AuthenticatedIdentity,
    remove: RemoveDeployment,
    resource: CapabilitySlotKey,
    expected_instance_version: u64,
    mode: Mode,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Original,
    Resume,
}

impl PreparedInstanceDrainCoordinator {
    /// Derives all concrete ports from the SAME retained controller and its pools.
    ///
    /// This performs no bootstrap, provider clone, database IO or filesystem writes.
    ///
    /// # Errors
    /// Rejects changed original owners/configuration or invalid trusted policy data.
    pub fn new(
        controller: Arc<ManagedInvocationController>,
    ) -> Result<Self, PreparedInstanceDrainError> {
        controller.validate_owners()?;
        let inputs = controller.drain_inputs();
        let configuration = &inputs.configuration;
        let scope = configuration.provider_scope();
        let platform = PlatformPolicyObservation::new(
            configuration.platform_policy().clone(),
            configuration.platform_version().to_owned(),
        )?;
        let resume = RemoveResumeConfiguration::new(
            scope.namespace().to_owned(),
            scope.host_id().to_owned(),
            platform.clone(),
            DeploymentExecutionProfile::RuntimeNamedV1,
        )?;
        let deployment = PostgresDeploymentRepository::new(inputs.pools.application.clone())
            .with_remove_resume(resume.clone());
        let preparation = PostgresExecutionPreparationRepository::new(
            inputs.pools.application.clone(),
            platform.clone(),
            DeploymentExecutionProfile::RuntimeNamedV1,
        )
        .with_remove_resume(resume.clone())?;
        let safety = PostgresPreparedInstanceSafetyRepository::new(
            inputs.pools.application.clone(),
            inputs.pools.worker.clone(),
            scope,
            platform,
        )
        .with_remove_resume(resume)?;
        let reader = PgInstanceDrainReader::new(inputs.pools.worker.clone(), configuration.clone());
        let recorder = PostgresEffectVerificationRecorder::new(inputs.pools.worker.clone());
        Ok(Self {
            context: Arc::new(Context {
                controller,
                deployment,
                preparation,
                safety,
                reader,
                recorder,
            }),
            pending: Arc::new(Mutex::new(BTreeMap::new())),
        })
    }

    /// Admits an original Remove and drives only this named instance's Drain.
    ///
    /// Original claim/closure inputs move into registered ownership before any
    /// await. Dropping the waiter does not abort admitted execution. Existing
    /// unfinished originals keep their claim; errors retain their actual source.
    ///
    /// # Errors
    /// Denies current authority/CAS, changed originals, held history or cleanup.
    pub async fn drain(
        &self,
        identity: &AuthenticatedIdentity,
        remove: RemoveDeployment,
        resource: &CapabilitySlotKey,
        expected_instance_version: u64,
    ) -> Result<ResourceProgress, Arc<PreparedInstanceDrainError>> {
        custody::submit(
            self,
            Request {
                identity: identity.clone(),
                remove,
                resource: resource.clone(),
                expected_instance_version,
                mode: Mode::Original,
            },
        )
        .await
    }

    /// Resumes through a genuinely new current-manager Remove admission.
    ///
    /// Full durable closure history precedes physical work. Inactive completed
    /// Drain replay uses its protected observer without minting another claim,
    /// target or verification. Historical actors are never credentials.
    ///
    /// # Errors
    /// Denies changed/foreign history, current authority or unsupported continuation.
    pub async fn resume(
        &self,
        identity: &AuthenticatedIdentity,
        remove: RemoveDeployment,
        resource: &CapabilitySlotKey,
        expected_instance_version: u64,
    ) -> Result<ResourceProgress, Arc<PreparedInstanceDrainError>> {
        custody::submit(
            self,
            Request {
                identity: identity.clone(),
                remove,
                resource: resource.clone(),
                expected_instance_version,
                mode: Mode::Resume,
            },
        )
        .await
    }

    /// Reports registered held originals; observation grants no cleanup authority.
    #[must_use]
    pub fn pending_drains(&self) -> Vec<PreparedInstanceDrainPending> {
        custody::inspect(&self.pending)
    }
}
