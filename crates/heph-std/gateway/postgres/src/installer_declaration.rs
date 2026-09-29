//! Immutable gateway declaration persistence and installation helpers.

use super::{GatewayInstallError, InstallGatewayManifest, InstalledGateway};
use agent_config::{RepositoryGatewaysConfig, parse_repository_gateways};
use forge_domain::{ProjectId, RepositoryId};
use gateway_domain::{
    Exposure, GatewayDeclaration, GatewayId, GatewayRevisionId, HttpMethod, ServiceLogCaptureMode,
};
use identity_domain::AuthenticatedIdentity;
use release_domain::ReleaseId;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Exact released agent selected by a repository gateway declaration.
#[derive(Debug, Clone)]
pub struct ReleaseAgentBinding {
    pub id: Uuid,
    pub key: String,
}

pub async fn resolve_release_agent(
    tx: &mut Transaction<'_, Postgres>,
    command: &InstallGatewayManifest,
    declaration: &GatewayDeclaration,
) -> Result<ReleaseAgentBinding, GatewayInstallError> {
    let release_id = command.release_id.ok_or(GatewayInstallError::Unavailable)?;
    sqlx::query_as::<_, ReleaseAgentBindingRow>(
        "SELECT agent.id, agent.agent_key AS key
         FROM release_agents AS agent
         JOIN releases AS release ON release.id = agent.release_id
         WHERE agent.release_id = $1
           AND agent.agent_key = $2
           AND release.repository_id = $3
           AND release.state = 'published'",
    )
    .bind(release_id.as_uuid())
    .bind(&declaration.agent_name)
    .bind(command.repository_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await?
    .map(|row| ReleaseAgentBinding {
        id: row.id,
        key: row.key,
    })
    .ok_or(GatewayInstallError::Unavailable)
}

#[derive(sqlx::FromRow)]
pub struct PublishedGatewayReleaseRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub repository_id: Uuid,
    pub source_commit: String,
    pub state: String,
}

#[derive(sqlx::FromRow)]
struct ReleaseAgentBindingRow {
    id: Uuid,
    key: String,
}

pub fn parse_manifest(source: &[u8]) -> Result<RepositoryGatewaysConfig, GatewayInstallError> {
    let parsed = parse_repository_gateways(source);
    let config = parsed.config.ok_or(GatewayInstallError::InvalidManifest {
        diagnostics: parsed.diagnostics.clone(),
    })?;
    if config.gateways.is_empty() {
        return Err(GatewayInstallError::InvalidManifest {
            diagnostics: parsed.diagnostics,
        });
    }
    Ok(config)
}

pub async fn require_repository_boundary(
    tx: &mut Transaction<'_, Postgres>,
    command: &InstallGatewayManifest,
) -> Result<(), GatewayInstallError> {
    let repository_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM repositories WHERE id = $1 AND project_id = $2)",
    )
    .bind(command.repository_id.as_uuid())
    .bind(command.project_id.as_uuid())
    .fetch_one(&mut **tx)
    .await?;
    if !repository_exists {
        return Err(GatewayInstallError::Unavailable);
    }
    if let Some(release_id) = command.release_id {
        let release_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM releases
                 JOIN repositories ON repositories.id = releases.repository_id
                 WHERE releases.id = $1
                   AND releases.repository_id = $2
                   AND repositories.project_id = $3
                   AND releases.state = 'published'
             )",
        )
        .bind(release_id.as_uuid())
        .bind(command.repository_id.as_uuid())
        .bind(command.project_id.as_uuid())
        .fetch_one(&mut **tx)
        .await?;
        if !release_exists {
            return Err(GatewayInstallError::Unavailable);
        }
    }
    Ok(())
}

// Keep the declaration, route, and authority materialization in one atomic
// transaction despite the bounded number of immutable row fields.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub async fn install_declaration(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    project_id: ProjectId,
    repository_id: RepositoryId,
    release_id: Option<ReleaseId>,
    release_agent: ReleaseAgentBinding,
    declaration: GatewayDeclaration,
    normalized_hash: [u8; 32],
) -> Result<InstalledGateway, GatewayInstallError> {
    let gateway_id = GatewayId::from_uuid(
        sqlx::query_scalar(
            "INSERT INTO gateways (id, project_id, repository_id, name, lifecycle, created_by)
             VALUES ($1, $2, $3, $4, 'enabled', $5)
             ON CONFLICT (repository_id, name) DO UPDATE SET updated_at = now()
               WHERE gateways.lifecycle <> 'removed'
             RETURNING id",
        )
        .bind(GatewayId::new().as_uuid())
        .bind(project_id.as_uuid())
        .bind(repository_id.as_uuid())
        .bind(declaration.name.as_str())
        .bind(identity.user_id.as_uuid())
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(GatewayInstallError::Unavailable)?,
    );
    let revision_id = GatewayRevisionId::new();
    let inserted_revision: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, release_id, release_agent_id,
             release_agent_key, handler_contract, exposure, parameters, secret_slots,
             mailbox_slots, service_loopback_port, service_readiness_path,
             service_health_path, service_log_capture_mode, normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
         ON CONFLICT (gateway_id, normalized_hash) DO NOTHING
         RETURNING id",
    )
    .bind(revision_id.as_uuid())
    .bind(gateway_id.as_uuid())
    .bind(project_id.as_uuid())
    .bind(repository_id.as_uuid())
    .bind(release_id.map(ReleaseId::as_uuid))
    .bind(release_agent.id)
    .bind(release_agent.key)
    .bind(&declaration.handler_contract)
    .bind(exposure_name(declaration.exposure))
    .bind(&declaration.parameters)
    .bind(&declaration.secret_slots)
    .bind(
        declaration
            .mailbox_publication_slots
            .iter()
            .map(|slot| slot.key.as_str())
            .collect::<Vec<_>>(),
    )
    .bind(
        declaration
            .service
            .as_ref()
            .map(|service| i32::from(service.loopback_port)),
    )
    .bind(
        declaration
            .service
            .as_ref()
            .map(|service| service.readiness_path.as_str()),
    )
    .bind(
        declaration
            .service
            .as_ref()
            .map(|service| service.health_path.as_str()),
    )
    .bind(
        declaration
            .service
            .as_ref()
            .map_or(ServiceLogCaptureMode::Disabled, |service| {
                service.log_capture_mode
            })
            .as_str(),
    )
    .bind(normalized_hash.as_slice())
    .bind(identity.user_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await?;
    let revision_id =
        GatewayRevisionId::from_uuid(match inserted_revision {
            Some(id) => {
                for route in &declaration.routes {
                    sqlx::query(
                        "INSERT INTO gateway_routes
                        (id, gateway_revision_id, gateway_id, project_id, path, methods)
                     VALUES ($1, $2, $3, $4, $5, $6)",
                    )
                    .bind(Uuid::new_v4())
                    .bind(id)
                    .bind(gateway_id.as_uuid())
                    .bind(project_id.as_uuid())
                    .bind(route.path.as_str())
                    .bind(
                        route
                            .methods
                            .iter()
                            .map(|method| method_name(*method))
                            .collect::<Vec<_>>(),
                    )
                    .execute(&mut **tx)
                    .await?;
                }
                id
            }
            None => sqlx::query_scalar(
                "SELECT id FROM gateway_revisions WHERE gateway_id = $1 AND normalized_hash = $2",
            )
            .bind(gateway_id.as_uuid())
            .bind(normalized_hash.as_slice())
            .fetch_one(&mut **tx)
            .await?,
        });
    if declaration.handler_contract == "http.service.v1" {
        // A service declaration is durable desired state. It becomes
        // publicly serving only after a later readiness-gated activation.
        sqlx::query(
            "UPDATE gateways SET desired_service_revision_id = $2, updated_at = now()
             WHERE id = $1",
        )
        .bind(gateway_id.as_uuid())
        .bind(revision_id.as_uuid())
        .execute(&mut **tx)
        .await?;
    } else {
        // Stateless gateways retain their existing atomic immediate-activation
        // behavior and supersede any pending service declaration.
        sqlx::query(
            "UPDATE gateways SET active_revision_id = $2,
                    desired_service_revision_id = NULL, updated_at = now()
             WHERE id = $1",
        )
        .bind(gateway_id.as_uuid())
        .bind(revision_id.as_uuid())
        .execute(&mut **tx)
        .await?;
    }
    Ok(InstalledGateway {
        gateway_id,
        revision_id,
    })
}

pub fn installation_hash(
    declaration_hash: [u8; 32],
    release_id: Option<ReleaseId>,
    release_agent_id: Uuid,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"hephaestus-gateway-install-v1\0");
    digest.update(declaration_hash);
    if let Some(release_id) = release_id {
        digest.update(release_id.as_uuid().as_bytes());
    }
    digest.update(release_agent_id.as_bytes());
    digest.finalize().into()
}

pub const fn exposure_name(exposure: Exposure) -> &'static str {
    match exposure {
        Exposure::Public => "public",
        Exposure::HephAuthenticated => "heph_authenticated",
    }
}

pub const fn method_name(method: HttpMethod) -> &'static str {
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
