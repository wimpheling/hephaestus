//! Durable resource recipe admission, independent of provider execution.
//!
//! Catalog evidence is authoritative when admitted and immutable thereafter.
//! Historical hydration conveys no current grants. Installation always reloads
//! live source and external facts; cleanup does not require source access.

mod admission;
mod authorization;
mod catalog;
mod evidence;
mod hydration;
mod persistence;
mod receipts;
mod rows;

use sqlx::PgPool;

/// `PostgreSQL` adapter for install, inspect, and remove admission.
///
/// These inherent methods commit intent and receipts only. Provider effects and
/// the complete application repository trait are added when implemented.
#[derive(Clone)]
pub struct PostgresDeploymentRepository {
    pool: PgPool,
}

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
