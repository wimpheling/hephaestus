//! Durable resource recipe admission, independent of provider execution.
//!
//! Catalog evidence is authoritative when admitted and immutable thereafter.
//! Historical hydration conveys no current grants. Installation always reloads
//! live source and external facts; cleanup does not require source access.

mod admission;
mod authorization;
mod catalog;
mod effect_claim;
mod effect_context;
mod effect_outcome;
mod effect_state;
mod evidence;
mod execution_preparation;
mod execution_rows;
mod execution_transitions;
mod hydration;
mod owned_creation_preparation;
mod owned_partial_retention;
mod owned_ready_retention;
mod persistence;
mod planning;
mod planning_sources;
mod receipts;
mod remove_resume;
mod repository;
mod retirement;
mod rows;
mod terminal;
mod verification;

use sqlx::PgPool;

/// `PostgreSQL` adapter for install, inspect, and remove admission.
///
/// Repository methods commit intent, claims, and verified outcomes. Provider
/// execution happens outside these transactions through trusted worker adapters.
#[derive(Clone)]
pub struct PostgresDeploymentRepository {
    pool: PgPool,
    remove_resume: Option<recipe_application::RemoveResumeConfiguration>,
}

pub use execution_preparation::PostgresExecutionPreparationRepository;
pub use execution_rows::encode_effect_claim;
pub use owned_ready_retention::PostgresOwnedReadyRetentionRepository;
pub use planning::PostgresPlanningCatalog;
pub use verification::{PostgresEffectVerificationRecorder, VerifiedOutcome};

impl PostgresDeploymentRepository {
    /// Enables qualified fresh-manager continuation using trusted comparison data.
    ///
    /// This builder grants no runtime authority; every effect rechecks live admission.
    #[must_use]
    pub fn with_remove_resume(
        mut self,
        configuration: recipe_application::RemoveResumeConfiguration,
    ) -> Self {
        self.remove_resume = Some(configuration);
        self
    }

    /// Creates an admission ledger over an existing pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self {
            pool,
            remove_resume: None,
        }
    }
}

pub(crate) fn repository_error(_: impl std::fmt::Debug) -> recipe_application::DeploymentError {
    recipe_application::DeploymentError::Repository
}
