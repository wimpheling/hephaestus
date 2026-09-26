//! UI browser static and gateway resource projection.

use super::{PgUiBrowserServingStore, support::set_verified_actor_context};
use async_trait::async_trait;
use identity_domain::{RequestId, UserId};
use release_domain::{
    ContentHash, ReleaseArtifactId, UiInstallationGenerationId,
    ui::{UiCachePolicy, UiMediaType},
};
use release_service::UiBrowserSessionContext;
use release_service::ui_browser_serving::{
    UiBrowserHttpPath, UiBrowserHttpRequest, UiBrowserHttpServingProjection, UiGatewayRequestKind,
    UiGatewayRequestProjection, UiServingError, UiServingProjection, UiStaticArtifactProjection,
};
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

/// Candidate classification is expressed as data and delegates every
/// authorization decision to migration 0090's typed verifier. Static and
/// managed candidates pass the platform-relative form expected by the
/// verifier; API candidates retain the canonical absolute gateway path. The
/// security-definer projection returns the canonical base alias separately so
/// Rust can emit a redirect without rewriting an ordinary gateway path.
/// The count rejects legacy static/API or static/managed collisions rather
/// than choosing an arbitrary fallback.
#[derive(Debug, FromRow)]
struct ResourceRow {
    session_id: Uuid,
    parent_session_id: Uuid,
    actor_id: Uuid,
    organization_id: Uuid,
    installation_id: Uuid,
    generation_id: Uuid,
    session_route: String,
    expires_at: OffsetDateTime,
    canonical_path: String,
    matched_kind: String,
    artifact_id: Option<Uuid>,
    storage_key: Option<Uuid>,
    content_hash: Option<Vec<u8>>,
    size_bytes: Option<i64>,
    media_type: Option<String>,
    cache_policy: Option<String>,
}

#[async_trait]
impl UiBrowserHttpServingProjection for PgUiBrowserServingStore {
    async fn authenticate_and_project_http(
        &self,
        request_id: RequestId,
        session_secret: release_domain::ui_browser::UiBrowserSessionSecret,
        expected_generation_id: UiInstallationGenerationId,
        request: UiBrowserHttpRequest,
    ) -> Result<UiServingProjection, UiServingError> {
        let digest = session_secret.digest().as_bytes().to_vec();
        let path = request.path().as_str().to_owned();
        let method = http_method_name(request.method());
        let mut transaction = self
            .app_pool
            .begin()
            .await
            .map_err(|_| UiServingError::Unavailable)?;
        let row = sqlx::query_as::<_, ResourceRow>(include_str!(
            "../ui_browser_resources_projection.sql"
        ))
        .bind(&digest)
        .bind(expected_generation_id.as_uuid())
        .bind(&path)
        .bind(method)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| UiServingError::Unavailable)?
        .ok_or(UiServingError::Unauthenticated)?;
        set_verified_actor_context(&mut transaction, row.actor_id, request_id)
            .await
            .map_err(|_| UiServingError::Unavailable)?;
        transaction
            .commit()
            .await
            .map_err(|_| UiServingError::Unavailable)?;
        let context = context_from_row(&row)?;
        let request_path =
            UiBrowserHttpPath::parse(path).map_err(|_| UiServingError::Unavailable)?;
        let canonical_path = UiBrowserHttpPath::parse(row.canonical_path.clone())
            .map_err(|_| UiServingError::Unavailable)?;
        let needs_redirect = matches!(row.matched_kind.as_str(), "static" | "managed_service")
            && canonical_path.as_str() != request_path.as_str();
        match row.matched_kind.as_str() {
            "static" | "managed_service" if needs_redirect => Ok(UiServingProjection::Redirect {
                context,
                location: canonical_path,
            }),
            "static" => Ok(UiServingProjection::Static {
                context,
                artifact: artifact_from_row(&row)?,
            }),
            "managed_service" | "api" => Ok(UiServingProjection::Gateway {
                context,
                request: UiGatewayRequestProjection {
                    kind: match row.matched_kind.as_str() {
                        "managed_service" => UiGatewayRequestKind::Managed,
                        "api" => UiGatewayRequestKind::Api,
                        _ => return Err(UiServingError::Unavailable),
                    },
                    path: request_path,
                    method: request.method(),
                },
            }),
            _ => Err(UiServingError::Unavailable),
        }
    }
}

fn context_from_row(row: &ResourceRow) -> Result<UiBrowserSessionContext, UiServingError> {
    let route = release_domain::ui_browser::UiBrowserRoute::parse(row.session_route.clone())
        .map_err(|_| UiServingError::Unavailable)?;
    Ok(UiBrowserSessionContext {
        session_id: release_domain::ui_browser::UiBrowserSessionId::from_uuid(row.session_id),
        parent_session_id: identity_domain::BrowserSessionId::from_uuid(row.parent_session_id),
        actor_id: UserId::from_uuid(row.actor_id),
        organization_id: forge_domain::OrganizationId::from_uuid(row.organization_id),
        installation_id: release_domain::UiInstallationId::from_uuid(row.installation_id),
        generation_id: UiInstallationGenerationId::from_uuid(row.generation_id),
        route,
        expires_at: row.expires_at,
    })
}

fn artifact_from_row(row: &ResourceRow) -> Result<UiStaticArtifactProjection, UiServingError> {
    let digest: [u8; 32] = row
        .content_hash
        .as_deref()
        .ok_or(UiServingError::Unavailable)?
        .try_into()
        .map_err(|_| UiServingError::Unavailable)?;
    let size_bytes = u64::try_from(row.size_bytes.ok_or(UiServingError::Unavailable)?)
        .map_err(|_| UiServingError::Unavailable)?;
    Ok(UiStaticArtifactProjection {
        artifact_id: ReleaseArtifactId::from_uuid(
            row.artifact_id.ok_or(UiServingError::Unavailable)?,
        ),
        storage_key: row.storage_key.ok_or(UiServingError::Unavailable)?,
        content_hash: ContentHash::from_digest(digest),
        size_bytes,
        media_type: UiMediaType::try_from(
            row.media_type.clone().ok_or(UiServingError::Unavailable)?,
        )
        .map_err(|_| UiServingError::Unavailable)?,
        cache_policy: match row.cache_policy.as_deref() {
            Some("no_store") => UiCachePolicy::NoStore,
            _ => return Err(UiServingError::Unavailable),
        },
    })
}

const fn http_method_name(method: gateway_domain::HttpMethod) -> &'static str {
    match method {
        gateway_domain::HttpMethod::Get => "GET",
        gateway_domain::HttpMethod::Post => "POST",
        gateway_domain::HttpMethod::Put => "PUT",
        gateway_domain::HttpMethod::Patch => "PATCH",
        gateway_domain::HttpMethod::Delete => "DELETE",
        gateway_domain::HttpMethod::Head => "HEAD",
        gateway_domain::HttpMethod::Options => "OPTIONS",
    }
}
