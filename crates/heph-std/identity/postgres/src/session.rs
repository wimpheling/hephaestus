//! `PostgreSQL` browser-session lifecycle adapter.

mod authenticate;
mod create;
mod revoke;
mod traits;

use sqlx::PgPool;

/// PostgreSQL-backed browser-session lifecycle operations.
///
/// Creation uses the worker-authorized pool, while verification uses a
/// separate application-role pool and the migration's security-definer
/// function.
#[derive(Clone)]
pub struct PostgresBrowserSessionStore {
    worker_pool: PgPool,
    application_pool: PgPool,
}

impl PostgresBrowserSessionStore {
    /// Creates a store with separate creation and application verification pools.
    ///
    /// Creation uses the worker role because it writes the session row.
    /// Verification uses only the application role and the migration's
    /// security-definer function; it never receives table access or a worker
    /// fallback.
    #[must_use]
    pub const fn new(worker_pool: PgPool, application_pool: PgPool) -> Self {
        Self {
            worker_pool,
            application_pool,
        }
    }
}
