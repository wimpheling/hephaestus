use forge_domain::OrganizationId;
use gateway_domain::HttpMethod;
use identity_domain::{BrowserSessionId, UserId};
use release_domain::ui_browser::{UiBrowserRoute, UiBrowserSessionId};
use release_domain::{UiInstallationGenerationId, UiInstallationId};
use release_service::{
    AuthenticateUiBrowserSession, UiBrowserRequestRoute, UiBrowserSessionContext,
    UiBrowserSessionError,
};
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

impl super::PgUiBrowserSessionStore {
    /// Authenticates one child bearer through the application-role verifier.
    ///
    /// The verifier derives actor and organization from the stored child row;
    /// this method supplies no caller actor context and returns only safe
    /// generation-bound metadata.
    ///
    /// # Errors
    ///
    /// Returns `Unavailable` when the verifier or stored route cannot be read,
    /// and `Unauthenticated` when no valid child bearer matches the request.
    pub async fn authenticate_ui_browser_session(
        &self,
        command: AuthenticateUiBrowserSession,
    ) -> Result<UiBrowserSessionContext, UiBrowserSessionError> {
        let (request_kind, request_path, request_method) = match command.request_route {
            UiBrowserRequestRoute::Static { route } => ("static", route.as_str().to_owned(), "GET"),
            UiBrowserRequestRoute::Managed { route } => {
                ("managed_service", route.as_str().to_owned(), "GET")
            }
            UiBrowserRequestRoute::Api { route, method } => {
                ("api", route.as_str().to_owned(), http_method_name(method))
            }
        };
        let session_digest = command.session_secret.digest().as_bytes().to_vec();
        let row = sqlx::query_as::<_, AuthenticatedUiBrowserSessionRow>(
            "SELECT session_id, parent_session_id, actor_id, organization_id,
                    installation_id, generation_id, route, expires_at
             FROM authenticate_ui_browser_session($1, $2, $3, $4, $5)",
        )
        .bind(session_digest)
        .bind(command.expected_generation_id.as_uuid())
        .bind(request_kind)
        .bind(request_path)
        .bind(request_method)
        .fetch_optional(&self.app_pool)
        .await
        .map_err(|_| UiBrowserSessionError::Unavailable)?
        .ok_or(UiBrowserSessionError::Unauthenticated)?;
        let route =
            UiBrowserRoute::parse(row.route).map_err(|_| UiBrowserSessionError::Unavailable)?;
        Ok(UiBrowserSessionContext {
            session_id: UiBrowserSessionId::from_uuid(row.session_id),
            parent_session_id: BrowserSessionId::from_uuid(row.parent_session_id),
            actor_id: UserId::from_uuid(row.actor_id),
            organization_id: OrganizationId::from_uuid(row.organization_id),
            installation_id: UiInstallationId::from_uuid(row.installation_id),
            generation_id: UiInstallationGenerationId::from_uuid(row.generation_id),
            route,
            expires_at: row.expires_at,
        })
    }
}

const fn http_method_name(method: HttpMethod) -> &'static str {
    match method {
        HttpMethod::Get => "GET",
        HttpMethod::Post => "POST",
        HttpMethod::Put => "PUT",
        HttpMethod::Patch => "PATCH",
        HttpMethod::Delete => "DELETE",
        HttpMethod::Head => "HEAD",
        HttpMethod::Options => "OPTIONS",
    }
}

#[derive(Debug, FromRow)]
struct AuthenticatedUiBrowserSessionRow {
    session_id: Uuid,
    parent_session_id: Uuid,
    actor_id: Uuid,
    organization_id: Uuid,
    installation_id: Uuid,
    generation_id: Uuid,
    route: String,
    expires_at: OffsetDateTime,
}
