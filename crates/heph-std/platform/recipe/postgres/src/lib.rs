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
mod execution_rows;
mod execution_transitions;
mod hydration;
mod persistence;
mod planning;
mod planning_sources;
mod receipts;
mod repository;
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
}

pub use planning::PostgresPlanningCatalog;
pub use verification::{PostgresEffectVerificationRecorder, VerifiedOutcome};

impl PostgresDeploymentRepository {
    /// Creates an admission ledger over an existing pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

pub(crate) fn repository_error(_: impl std::fmt::Debug) -> recipe_application::DeploymentError {
    recipe_application::DeploymentError::Repository
}
