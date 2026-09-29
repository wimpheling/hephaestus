//! Active UI generation host resolution.

use async_trait::async_trait;
use release_domain::UiInstallationGenerationId;
use release_service::ui_browser_host::UiGenerationHost;
use release_service::ui_browser_serving::{
    ActiveUiGenerationHost, UiGenerationHostResolver, UiHostLookupError,
};
use sqlx::PgPool;
use uuid::Uuid;

/// App-pool implementation of the metadata-free active-generation host read.
#[derive(Clone)]
pub struct PgUiGenerationHostResolver {
    app_pool: PgPool,
}

impl PgUiGenerationHostResolver {
    /// Creates the resolver over an application-role pool.
    #[must_use]
    pub const fn new(app_pool: PgPool) -> Self {
        Self { app_pool }
    }
}

#[async_trait]
impl UiGenerationHostResolver for PgUiGenerationHostResolver {
    async fn resolve_active_generation_host(
        &self,
        host: UiGenerationHost,
    ) -> Result<Option<ActiveUiGenerationHost>, UiHostLookupError> {
        let generation_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT generation_id FROM public.resolve_active_ui_generation_host($1)",
        )
        .bind(host.generation_id().as_uuid())
        .fetch_optional(&self.app_pool)
        .await
        .map_err(|_| UiHostLookupError::Unavailable)?;
        Ok(generation_id.map(|generation_id| ActiveUiGenerationHost {
            generation_id: UiInstallationGenerationId::from_uuid(generation_id),
        }))
    }
}
