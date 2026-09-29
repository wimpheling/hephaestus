//! Worker-side UI browser handoff issuance adapter.
//!
//! This file contains the worker-side issuance and exchange methods only.
//! The application-role verifier is a separate, read-only method on the same
//! store; worker writes never use the application pool.

use async_trait::async_trait;
use authz_postgres::PostgresMelangeAuthorizer;
use release_service::{
    AuthenticateUiBrowserSession, CreateUiBrowserHandoff, CreatedUiBrowserHandoff,
    CreatedUiBrowserSession, ExchangeUiBrowserHandoff, UiBrowserHandoffError,
    UiBrowserSessionContext, UiBrowserSessionError, UiBrowserSessionStore,
};
use sqlx::PgPool;

mod audit;
mod authentication;
mod authorization;
mod eligibility;
mod exchange;
mod issuance;
mod rows;

/// `PostgreSQL` store for trusted UI browser handoff issuance and exchange.
pub struct PgUiBrowserSessionStore {
    worker_pool: PgPool,
    app_pool: PgPool,
    authorizer: PostgresMelangeAuthorizer,
}
impl PgUiBrowserSessionStore {
    /// Creates a store with separate worker and application pools.
    #[must_use]
    pub const fn new(worker_pool: PgPool, app_pool: PgPool) -> Self {
        Self {
            worker_pool,
            app_pool,
            authorizer: PostgresMelangeAuthorizer,
        }
    }
}

#[async_trait]
impl UiBrowserSessionStore for PgUiBrowserSessionStore {
    async fn create_ui_browser_handoff(
        &self,
        command: CreateUiBrowserHandoff,
    ) -> Result<CreatedUiBrowserHandoff, UiBrowserHandoffError> {
        self.create_ui_browser_handoff(command).await
    }

    async fn exchange_ui_browser_handoff(
        &self,
        command: ExchangeUiBrowserHandoff,
    ) -> Result<CreatedUiBrowserSession, UiBrowserHandoffError> {
        self.exchange_ui_browser_handoff(command).await
    }

    async fn authenticate_ui_browser_session(
        &self,
        command: AuthenticateUiBrowserSession,
    ) -> Result<UiBrowserSessionContext, UiBrowserSessionError> {
        self.authenticate_ui_browser_session(command).await
    }
}
