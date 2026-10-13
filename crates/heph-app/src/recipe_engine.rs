//! Injected installation orchestration over checked preparation and owning adapters.
//!
//! This backend is opt-in through construction. It does not route RPCs, start
//! workloads, drain instances or initialize roots. Retained external volumes
//! require explicit injection of the qualified verification boundary.

mod activation;
mod errors;
mod execution;
mod external;
mod inspection;
mod removal;
pub use removal::{PreparedRecipeRemovalCoordinator, RecipeRemovalError, RecipeRemovalPending};
mod validation;

pub use errors::RecipeApplicationError;
pub use inspection::{
    ActivationInspection, PreparedRecipeInspection, RecipeActivationAttempt, RecipeInstallation,
};

use std::sync::Arc;

use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentId, DeploymentOperation, InstallDeployment, PlanningRequest,
    RecipePlanner,
};
use recipe_postgres::{
    PostgresDeploymentRepository, PostgresExecutionPreparationRepository, PostgresPlanningCatalog,
};
use release_service::InstanceExecutionService;

use crate::{
    recipe_instance_import::PreparedInstanceImporter,
    recipe_owned_volume::PreparedOwnedVolumeCreator,
    recipe_retained_volume::RetainedVolumeVerifier,
};

/// Executes one saved installation using freshly authorized owning services.
///
/// All injected adapters must share the same database, frozen platform policy and
/// trusted roots. This value constructs neither identities nor owner authority.
/// Instance activation is a separate checked admission after original creation.
#[derive(Clone)]
pub struct PreparedRecipeApplication {
    planner: Arc<RecipePlanner<PostgresPlanningCatalog>>,
    repository: Arc<PostgresDeploymentRepository>,
    preparation: Arc<PostgresExecutionPreparationRepository>,
    volumes: Arc<PreparedOwnedVolumeCreator>,
    instances: Arc<PreparedInstanceImporter>,
    activation: Arc<dyn InstanceExecutionService>,
    retained_volumes: Option<Arc<dyn RetainedVolumeVerifier>>,
}

impl PreparedRecipeApplication {
    /// Injects the actual planning, ledger, owning providers and activation boundary.
    #[must_use]
    pub const fn new(
        planner: Arc<RecipePlanner<PostgresPlanningCatalog>>,
        repository: Arc<PostgresDeploymentRepository>,
        preparation: Arc<PostgresExecutionPreparationRepository>,
        volumes: Arc<PreparedOwnedVolumeCreator>,
        instances: Arc<PreparedInstanceImporter>,
        activation: Arc<dyn InstanceExecutionService>,
    ) -> Self {
        Self {
            planner,
            repository,
            preparation,
            volumes,
            instances,
            activation,
            retained_volumes: None,
        }
    }

    /// Enables only retained owned-volume External verification through its owning adapter.
    ///
    /// The injected port must use this engine's actual database, policy, roles and
    /// trusted physical/provider ownership. It independently qualifies retained
    /// lineage, native custody and the atomically paired actor completion.
    #[must_use]
    pub fn with_retained_volume_verifier(
        mut self,
        verifier: Arc<dyn RetainedVolumeVerifier>,
    ) -> Self {
        self.retained_volumes = Some(verifier);
        self
    }

    /// Plans and admits exact immutable input, then installs in dependency order.
    ///
    /// A retry must use the same logical identity. Changed input is a conflict;
    /// cancellation and storage errors never permit replacement resource IDs.
    ///
    /// # Errors
    /// Returns live denial, input drift or a held owning-adapter/activation result.
    /// Without explicit retained verification, External resources deny before
    /// provider effects. Other External kinds remain unsupported when configured.
    pub async fn install(
        &self,
        identity: &AuthenticatedIdentity,
        request: &PlanningRequest,
    ) -> Result<RecipeInstallation, RecipeApplicationError> {
        let planned = self.planner.plan(identity, request).await?;
        let command = CommandIdentity::from_identity(identity, DeploymentOperation::Install)?;
        let admitted = self
            .repository
            .admit_install(
                identity,
                InstallDeployment {
                    command,
                    intent: planned.intent().clone(),
                },
            )
            .await?;
        Box::pin(self.execute(identity, admitted)).await
    }

    /// Resumes original immutable intent under an actual fresh authenticated request.
    ///
    /// No parameters, resolved graph, attempt IDs or provider IDs can be replaced.
    /// This does not reconstruct an authenticated identity from recorded actor data.
    ///
    /// # Errors
    /// Rejects changed actor/logical identity, removal, source/configuration drift,
    /// and unproven outcomes retained by the owning resource adapters.
    pub async fn resume(
        &self,
        identity: &AuthenticatedIdentity,
        original: CommandIdentity,
        deployment: DeploymentId,
    ) -> Result<RecipeInstallation, RecipeApplicationError> {
        original.validate(identity, DeploymentOperation::Install)?;
        let stored = self.repository.inspect(identity, deployment).await?;
        let admitted = self
            .repository
            .admit_install(
                identity,
                InstallDeployment {
                    command: original,
                    intent: stored.intent,
                },
            )
            .await?;
        Box::pin(self.execute(identity, admitted)).await
    }

    /// Reads the durable ledger and historical preparation without effect authority.
    ///
    /// Activation is explicitly unobserved: Ready does not imply an open gate.
    ///
    /// # Errors
    /// Rejects denied project inspection or inconsistent persisted preparation.
    pub async fn inspect(
        &self,
        identity: &AuthenticatedIdentity,
        deployment: DeploymentId,
    ) -> Result<PreparedRecipeInspection, RecipeApplicationError> {
        inspection::read(self, identity, deployment).await
    }
}
