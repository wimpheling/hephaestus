use authz_postgres::PostgresMelangeAuthorizer;
use sqlx::PgPool;
use std::sync::Arc;

mod builds;
mod helpers;
mod images;
mod inspect;
mod outbox;
mod projects;
mod receive;
mod receive_inner;
mod replay;
mod repository_ops;
mod revisions;
mod rows;
mod triggers;
mod ui_manifest;
mod ui_manifest_store;
mod validation;

pub use helpers::{git, serialization, storage};

/// `PostgreSQL` forge metadata and receive repository.
#[derive(Clone)]
pub struct PgForgeRepository {
    pub(crate) pool: PgPool,
    pub(crate) storage: Arc<crate::GitStorage>,
    pub(crate) authorizer: Option<Arc<PostgresMelangeAuthorizer>>,
}

impl PgForgeRepository {
    /// Creates a repository service.
    #[must_use]
    pub const fn new(pool: PgPool, storage: Arc<crate::GitStorage>) -> Self {
        Self {
            pool,
            storage,
            authorizer: None,
        }
    }

    /// Enables transaction-native run authorization for authenticated receives.
    #[must_use]
    pub fn with_authorizer(mut self, authorizer: Arc<PostgresMelangeAuthorizer>) -> Self {
        self.authorizer = Some(authorizer);
        self
    }
}

#[cfg(test)]
mod tests;
