//! UI browser target and repository Git authorization projections.

use super::{PgUiBrowserServingStore, support::set_verified_actor_context};
use async_trait::async_trait;
use forge_domain::RepositoryId;
use identity_domain::{RequestId, UserId};
use release_domain::UiInstallationGenerationId;
use release_service::UiBrowserSessionContext;
use release_service::ui_browser_serving::{
    UiBrowserRepositoryGitAuthorization, UiGitAuthorizationError, UiRepositoryGitAuthorization,
    UiRepositoryGitOperation,
};
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, FromRow)]
struct GitAuthorizationRow {
    session_id: Uuid,
    parent_session_id: Uuid,
    actor_id: Uuid,
    organization_id: Uuid,
    installation_id: Uuid,
    generation_id: Uuid,
    repository_id: Uuid,
    access: String,
    session_route: String,
    expires_at: OffsetDateTime,
}

#[derive(Debug, FromRow)]
struct TargetContextRow {
    session_id: Uuid,
    parent_session_id: Uuid,
    actor_id: Uuid,
    organization_id: Uuid,
    installation_id: Uuid,
    generation_id: Uuid,
    repository_id: Uuid,
    session_route: String,
    expires_at: OffsetDateTime,
}

#[async_trait]
impl release_service::UiBrowserTargetContextProjection for PgUiBrowserServingStore {
    async fn project_ui_target(
        &self,
        request_id: RequestId,
        session_secret: release_domain::ui_browser::UiBrowserSessionSecret,
        expected_generation_id: UiInstallationGenerationId,
    ) -> Result<release_service::UiBrowserTargetContext, release_service::UiTargetContextError>
    {
        let mut transaction = self
            .app_pool
            .begin()
            .await
            .map_err(|_| release_service::UiTargetContextError::Unavailable)?;
        let row = sqlx::query_as::<_, TargetContextRow>(
            "SELECT session_id, parent_session_id, actor_id, organization_id,
                    installation_id, generation_id, repository_id, session_route,
                    expires_at
             FROM public.resolve_ui_browser_repository_target_context($1, $2)",
        )
        .bind(session_secret.digest().as_bytes().as_slice())
        .bind(expected_generation_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| release_service::UiTargetContextError::Unavailable)?
        .ok_or(release_service::UiTargetContextError::Unauthorized)?;
        set_verified_actor_context(&mut transaction, row.actor_id, request_id)
            .await
            .map_err(|_| release_service::UiTargetContextError::Unavailable)?;
        transaction
            .commit()
            .await
            .map_err(|_| release_service::UiTargetContextError::Unavailable)?;
        let route = release_domain::ui_browser::UiBrowserRoute::parse(row.session_route)
            .map_err(|_| release_service::UiTargetContextError::Unavailable)?;
        let context = UiBrowserSessionContext {
            session_id: release_domain::ui_browser::UiBrowserSessionId::from_uuid(row.session_id),
            parent_session_id: identity_domain::BrowserSessionId::from_uuid(row.parent_session_id),
            actor_id: UserId::from_uuid(row.actor_id),
            organization_id: forge_domain::OrganizationId::from_uuid(row.organization_id),
            installation_id: release_domain::UiInstallationId::from_uuid(row.installation_id),
            generation_id: UiInstallationGenerationId::from_uuid(row.generation_id),
            route,
            expires_at: row.expires_at,
        };
        Ok(release_service::UiBrowserTargetContext {
            context,
            repository_id: RepositoryId::from_uuid(row.repository_id),
        })
    }
}

#[async_trait]
impl UiBrowserRepositoryGitAuthorization for PgUiBrowserServingStore {
    async fn authorize_repository_git(
        &self,
        request_id: RequestId,
        session_secret: release_domain::ui_browser::UiBrowserSessionSecret,
        expected_generation_id: UiInstallationGenerationId,
        repository_id: RepositoryId,
        operation: UiRepositoryGitOperation,
    ) -> Result<UiRepositoryGitAuthorization, UiGitAuthorizationError> {
        let mut transaction = self
            .app_pool
            .begin()
            .await
            .map_err(|_| UiGitAuthorizationError::Unavailable)?;
        let row = sqlx::query_as::<_, GitAuthorizationRow>(
            "SELECT session_id, parent_session_id, actor_id, organization_id,
                    installation_id, generation_id, repository_id, access,
                    session_route, expires_at
             FROM public.resolve_ui_browser_repository_git_context($1, $2, $3, $4)",
        )
        .bind(session_secret.digest().as_bytes().as_slice())
        .bind(expected_generation_id.as_uuid())
        .bind(repository_id.as_uuid())
        .bind(operation.as_str())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| UiGitAuthorizationError::Unavailable)?
        .ok_or(UiGitAuthorizationError::Unauthorized)?;
        let access = release_domain::ui::UiRepositoryGitAccess::parse(row.access)
            .map_err(|_| UiGitAuthorizationError::Unavailable)?;
        set_verified_actor_context(&mut transaction, row.actor_id, request_id)
            .await
            .map_err(|_| UiGitAuthorizationError::Unavailable)?;
        transaction
            .commit()
            .await
            .map_err(|_| UiGitAuthorizationError::Unavailable)?;
        let route = release_domain::ui_browser::UiBrowserRoute::parse(row.session_route)
            .map_err(|_| UiGitAuthorizationError::Unavailable)?;
        let context = UiBrowserSessionContext {
            session_id: release_domain::ui_browser::UiBrowserSessionId::from_uuid(row.session_id),
            parent_session_id: identity_domain::BrowserSessionId::from_uuid(row.parent_session_id),
            actor_id: UserId::from_uuid(row.actor_id),
            organization_id: forge_domain::OrganizationId::from_uuid(row.organization_id),
            installation_id: release_domain::UiInstallationId::from_uuid(row.installation_id),
            generation_id: UiInstallationGenerationId::from_uuid(row.generation_id),
            route,
            expires_at: row.expires_at,
        };
        Ok(UiRepositoryGitAuthorization {
            actor_id: UserId::from_uuid(row.actor_id),
            repository_id: RepositoryId::from_uuid(row.repository_id),
            access,
            context,
        })
    }
}
