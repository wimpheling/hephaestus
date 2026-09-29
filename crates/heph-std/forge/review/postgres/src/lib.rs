//! `PostgreSQL` persistence adapters for review controls and command outbox.

#[path = "review/commands.rs"]
mod commands;
#[path = "review/models.rs"]
mod models;
#[path = "review/outbox.rs"]
mod outbox;
#[path = "review/repository.rs"]
mod repository;
#[path = "review/support.rs"]
mod support;
#[path = "review/transaction.rs"]
mod transaction;

use async_trait::async_trait;
use authz_postgres::PostgresMelangeAuthorizer;
use forge_domain::RepositoryId;
use forge_service::GitStorage;
use review_service::RepositoryLocator;
use sqlx::PgPool;
use std::sync::Arc;

const CANCEL_RUN_SUBJECT: &str = "heph.run.command.cancel.v1";
const START_RUN_SUBJECT: &str = "hephaestus.run.start";

/// PostgreSQL-backed review repository implementing the provider-neutral service port.
#[derive(Clone)]
pub struct PostgresReviewRepository {
    pool: PgPool,
    authorizer: PostgresMelangeAuthorizer,
}

impl PostgresReviewRepository {
    /// Creates an adapter over the supplied `PostgreSQL` pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self {
            pool,
            authorizer: PostgresMelangeAuthorizer,
        }
    }

    /// Returns the underlying pool for adapter composition and health checks.
    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }
}

/// Canonical Git repository locator backed by forge storage.
#[derive(Clone)]
pub struct GitRepositoryLocator {
    storage: Arc<GitStorage>,
}

impl GitRepositoryLocator {
    /// Creates a locator over canonical bare-Git storage.
    #[must_use]
    pub const fn new(storage: Arc<GitStorage>) -> Self {
        Self { storage }
    }
}

#[async_trait]
impl RepositoryLocator for GitRepositoryLocator {
    async fn locate(&self, repository_id: RepositoryId) -> Result<std::path::PathBuf, String> {
        self.storage
            .validate_existing(repository_id)
            .await
            .map_err(|error| error.to_string())
    }
}
