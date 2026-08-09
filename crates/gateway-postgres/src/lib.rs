//! `PostgreSQL` installation authority for repository-declared HTTP gateways.
//!
//! A caller supplies the exact repository manifest bytes from the release it
//! is installing. This adapter parses that source before opening its
//! transaction, authorizes project management in that transaction, and then
//! atomically installs only immutable declaration revisions and their routes.
//! It deliberately does not open a listener or derive provider configuration.

use agent_config::{Diagnostic, RepositoryGatewaysConfig, parse_repository_gateways};
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use forge_domain::{ProjectId, RepositoryId};
use gateway_domain::{Exposure, GatewayDeclaration, GatewayId, GatewayRevisionId, HttpMethod};
use identity_domain::AuthenticatedIdentity;
use release_domain::ReleaseId;
use sqlx::{PgPool, Postgres, Transaction};
use std::sync::Arc;
use uuid::Uuid;

/// Trusted request to install all valid declarations in one exact manifest.
#[derive(Debug, Clone)]
pub struct InstallGatewayManifest {
    /// Project which owns the repository and durable gateway identities.
    pub project_id: ProjectId,
    /// Repository containing the manifest.
    pub repository_id: RepositoryId,
    /// Optional immutable release whose source supplied this manifest.
    pub release_id: Option<ReleaseId>,
    /// Exact bytes from the repository-root `heph.gateways.toml` file.
    pub manifest: Vec<u8>,
}

/// One installed stable gateway and its selected immutable revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstalledGateway {
    /// Durable repository-scoped gateway identity.
    pub gateway_id: GatewayId,
    /// Immutable declaration selected as active by this installation.
    pub revision_id: GatewayRevisionId,
}

/// Result of atomically installing one manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallGatewayManifestResult {
    /// Gateways installed in source declaration order.
    pub gateways: Vec<InstalledGateway>,
}

/// Safe installation failure. Manifest diagnostics are intentionally retained
/// for trusted repository feedback; no database details are exposed.
#[derive(Debug, thiserror::Error)]
pub enum GatewayInstallError {
    /// The manifest did not pass the repository declaration contract.
    #[error("invalid gateway manifest")]
    InvalidManifest {
        /// Parser diagnostics suitable for repository feedback.
        diagnostics: Vec<Diagnostic>,
    },
    /// The caller cannot manage the target project.
    #[error("gateway installation is not authorized")]
    AuthorizationDenied,
    /// The repository, release, or project boundary is unavailable.
    #[error("gateway installation target is unavailable")]
    Unavailable,
    /// The durable gateway declaration could not be written.
    #[error("gateway installation persistence failed")]
    Persistence(#[from] sqlx::Error),
}

/// `PostgreSQL` control-plane service for immutable gateway installation.
#[derive(Clone)]
pub struct PostgresGatewayInstaller {
    pool: PgPool,
    authorizer: Arc<PostgresMelangeAuthorizer>,
}

impl PostgresGatewayInstaller {
    /// Creates an installer over the control-plane pool.
    #[must_use]
    pub const fn new(pool: PgPool, authorizer: Arc<PostgresMelangeAuthorizer>) -> Self {
        Self { pool, authorizer }
    }

    /// Parses, authorizes, and installs the exact source manifest.
    ///
    /// Reinstalling an identical declaration selects its existing immutable
    /// revision. Changed source creates a fresh revision and only then makes
    /// it active, so a failure cannot leave a partially installed gateway.
    ///
    /// # Errors
    ///
    /// Returns a safe manifest, authorization, boundary, or persistence
    /// failure without exposing provider configuration or listener state.
    pub async fn install(
        &self,
        identity: &AuthenticatedIdentity,
        command: InstallGatewayManifest,
    ) -> Result<InstallGatewayManifestResult, GatewayInstallError> {
        let config = parse_manifest(&command.manifest)?;
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require_manage(&mut tx, identity, command.project_id)
            .await?;
        require_repository_boundary(&mut tx, &command).await?;

        let mut installed = Vec::with_capacity(config.gateways.len());
        for configured in config.gateways {
            let declaration = configured
                .to_declaration()
                .map_err(|_| GatewayInstallError::Unavailable)?;
            let normalized_hash = declaration
                .validate()
                .map_err(|_| GatewayInstallError::Unavailable)?;
            installed.push(
                install_declaration(
                    &mut tx,
                    identity,
                    command.project_id,
                    command.repository_id,
                    command.release_id,
                    declaration,
                    normalized_hash,
                )
                .await?,
            );
        }
        tx.commit().await?;
        Ok(InstallGatewayManifestResult {
            gateways: installed,
        })
    }

    async fn require_manage(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        project_id: ProjectId,
    ) -> Result<(), GatewayInstallError> {
        let object = ObjectRef::new(ObjectType::Project, project_id.as_uuid());
        let decision = self
            .authorizer
            .check(
                tx,
                Subject::User(identity.user_id),
                Permission::CanManage,
                object,
            )
            .await
            .map_err(|_| GatewayInstallError::Unavailable)?;
        audit_decision(
            tx,
            identity.user_id,
            Permission::CanManage,
            object,
            decision,
            identity.request_id,
        )
        .await?;
        if decision == AuthorizationDecision::Allow {
            Ok(())
        } else {
            // Preserve denied authorization evidence even though the command
            // transaction rolls back its in-transaction audit row.
            let mut audit_tx = begin_actor_transaction(&self.pool, identity).await?;
            audit_decision(
                &mut audit_tx,
                identity.user_id,
                Permission::CanManage,
                object,
                decision,
                identity.request_id,
            )
            .await?;
            audit_tx.commit().await?;
            Err(GatewayInstallError::AuthorizationDenied)
        }
    }
}

fn parse_manifest(source: &[u8]) -> Result<RepositoryGatewaysConfig, GatewayInstallError> {
    let parsed = parse_repository_gateways(source);
    parsed.config.ok_or(GatewayInstallError::InvalidManifest {
        diagnostics: parsed.diagnostics,
    })
}

async fn require_repository_boundary(
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
            "SELECT EXISTS (SELECT 1 FROM releases WHERE id = $1 AND repository_id = $2)",
        )
        .bind(release_id.as_uuid())
        .bind(command.repository_id.as_uuid())
        .fetch_one(&mut **tx)
        .await?;
        if !release_exists {
            return Err(GatewayInstallError::Unavailable);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn install_declaration(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    project_id: ProjectId,
    repository_id: RepositoryId,
    release_id: Option<ReleaseId>,
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
            (id, gateway_id, project_id, repository_id, release_id, handler_contract,
             exposure, parameters, secret_slots, normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         ON CONFLICT (gateway_id, normalized_hash) DO NOTHING
         RETURNING id",
    )
    .bind(revision_id.as_uuid())
    .bind(gateway_id.as_uuid())
    .bind(project_id.as_uuid())
    .bind(repository_id.as_uuid())
    .bind(release_id.map(ReleaseId::as_uuid))
    .bind(&declaration.handler_contract)
    .bind(exposure_name(declaration.exposure))
    .bind(&declaration.parameters)
    .bind(&declaration.secret_slots)
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
    sqlx::query("UPDATE gateways SET active_revision_id = $2, updated_at = now() WHERE id = $1")
        .bind(gateway_id.as_uuid())
        .bind(revision_id.as_uuid())
        .execute(&mut **tx)
        .await?;
    Ok(InstalledGateway {
        gateway_id,
        revision_id,
    })
}

const fn exposure_name(exposure: Exposure) -> &'static str {
    match exposure {
        Exposure::Public => "public",
        Exposure::HephAuthenticated => "heph_authenticated",
    }
}

const fn method_name(method: HttpMethod) -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_validated_repository_gateway_source() {
        let source = br#"
version = 1

[[gateways]]
name = "telegram"
handler_contract = "http.v1"
exposure = "public"
parameters = {}

[[gateways.routes]]
path = "/telegram"
methods = ["POST"]
"#;
        let parsed = parse_manifest(source).expect("valid repository source");
        assert_eq!(parsed.gateways.len(), 1);
        assert!(parse_manifest(b"version = 2").is_err());
    }

    #[test]
    fn serializes_only_the_bounded_http_vocabulary() {
        assert_eq!(method_name(HttpMethod::Patch), "PATCH");
        assert_eq!(
            exposure_name(Exposure::HephAuthenticated),
            "heph_authenticated"
        );
    }
}
