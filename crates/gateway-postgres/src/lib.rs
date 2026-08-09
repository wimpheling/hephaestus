//! `PostgreSQL` installation authority for repository-declared HTTP gateways.
//!
//! A caller supplies the exact repository manifest bytes from the release it
//! is installing. This adapter parses that source before opening its
//! transaction, authorizes project management in that transaction, and then
//! atomically installs only immutable declaration revisions and their routes.
//! It deliberately does not open a listener or derive provider configuration.

use agent_config::{Diagnostic, RepositoryGatewaysConfig, parse_repository_gateways};
use async_trait::async_trait;
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use forge_domain::{ProjectId, RepositoryId};
use gateway_domain::{Exposure, GatewayDeclaration, GatewayId, GatewayRevisionId, HttpMethod};
use gateway_edge::{
    GatewayConfigRevision, GatewayDesiredConfiguration, GatewayEdgeError, GatewayInvocationOutcome,
    GatewayInvocationRecorder, GatewayLimits, GatewayRouteBinding, GatewayRouteResolver,
};
use http::Method;
use identity_domain::AuthenticatedIdentity;
use release_domain::ReleaseId;
use runtime_authority::{GatewayRuntimeAuthorityIssuer, GatewayRuntimeSessionRequest};
use sqlx::{PgPool, Postgres, Transaction};
use std::{sync::Arc, time::Duration};
use time::OffsetDateTime;
use uuid::Uuid;

/// Private worker adapter from authoritative gateway rows to the edge ports.
///
/// It never trusts a route identifier supplied by Caddy: resolution starts with
/// the canonical request path and selects only an enabled route of the active
/// immutable revision. Invocation rows retain correlation/lifecycle evidence
/// only; payloads remain at the private HTTP boundary.
#[derive(Clone)]
pub struct PostgresGatewayEdgeAuthority {
    pool: PgPool,
    limits: GatewayLimits,
    runtime_authority: Option<Arc<dyn GatewayRuntimeAuthorityIssuer>>,
    session_ttl: Duration,
}

impl PostgresGatewayEdgeAuthority {
    /// Creates the worker-side route and invocation adapter with explicit
    /// bounded HTTP limits.
    #[must_use]
    pub const fn new(pool: PgPool, limits: GatewayLimits) -> Self {
        Self {
            pool,
            limits,
            runtime_authority: None,
            session_ttl: Duration::from_secs(30),
        }
    }

    /// Attaches the generic gateway-session issuer used by a production
    /// dispatcher. The basic constructor remains useful for reconciliation
    /// workers that never accept public traffic.
    ///
    /// # Errors
    ///
    /// Returns an unavailable edge error when the requested session lifetime
    /// is zero and therefore cannot form a bounded session identity.
    pub fn with_runtime_authority(
        mut self,
        runtime_authority: Arc<dyn GatewayRuntimeAuthorityIssuer>,
        session_ttl: Duration,
    ) -> Result<Self, GatewayEdgeError> {
        if session_ttl.is_zero() {
            return Err(GatewayEdgeError::Unavailable);
        }
        self.runtime_authority = Some(runtime_authority);
        self.session_ttl = session_ttl;
        Ok(self)
    }

    /// Reconstructs the complete enabled desired route set from `PostgreSQL`.
    /// A caller supplies the derived revision after recording its desired hash;
    /// no provider configuration becomes authoritative.
    ///
    /// # Errors
    ///
    /// Returns an unavailable edge error when authoritative rows cannot be
    /// read or contain an invalid persisted HTTP vocabulary.
    pub async fn desired_configuration(
        &self,
        revision: GatewayConfigRevision,
    ) -> Result<GatewayDesiredConfiguration, GatewayEdgeError> {
        Ok(GatewayDesiredConfiguration {
            revision,
            routes: self.active_routes().await?,
        })
    }

    async fn active_routes(&self) -> Result<Vec<GatewayRouteBinding>, GatewayEdgeError> {
        let rows = sqlx::query_as::<_, ActiveRouteRow>(
            "SELECT route.id AS route_id, route.gateway_revision_id, route.path, route.methods
             FROM gateway_routes AS route
             JOIN gateways AS gateway ON gateway.id = route.gateway_id
             WHERE gateway.lifecycle = 'enabled'
               AND gateway.active_revision_id = route.gateway_revision_id
               AND route.enabled
             ORDER BY route.path, route.id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        rows.into_iter()
            .map(|row| active_route(row, self.limits))
            .collect()
    }
}

#[async_trait]
impl GatewayRouteResolver for PostgresGatewayEdgeAuthority {
    async fn resolve(
        &self,
        path_and_query: &str,
    ) -> Result<Option<GatewayRouteBinding>, GatewayEdgeError> {
        let path = canonical_request_path(path_and_query)?;
        let mut candidates = self.active_routes().await?;
        candidates.retain(|route| route_matches(route, path));
        candidates.sort_by(|left, right| {
            right
                .path_prefix
                .len()
                .cmp(&left.path_prefix.len())
                .then_with(|| left.route_id.cmp(&right.route_id))
        });
        Ok(candidates.into_iter().next())
    }
}

#[async_trait]
impl GatewayInvocationRecorder for PostgresGatewayEdgeAuthority {
    async fn accepted(
        &self,
        route: &GatewayRouteBinding,
        request_id: Uuid,
    ) -> Result<Uuid, GatewayEdgeError> {
        let invocation_id = Uuid::new_v4();
        let accepted = sqlx::query_as::<_, AcceptedInvocationRow>(
            "INSERT INTO gateway_invocations
                 (id, gateway_id, gateway_revision_id, gateway_route_id, project_id, request_id, outcome)
             SELECT $1, route.gateway_id, route.gateway_revision_id, route.id, route.project_id, $2, 'accepted'
             FROM gateway_routes AS route
             JOIN gateways AS gateway ON gateway.id = route.gateway_id
             WHERE route.id = $3
               AND route.gateway_revision_id = $4
               AND route.enabled
               AND gateway.lifecycle = 'enabled'
               AND gateway.active_revision_id = route.gateway_revision_id
             RETURNING gateway_id, gateway_revision_id",
        )
        .bind(invocation_id)
        .bind(request_id)
        .bind(route.route_id)
        .bind(route.gateway_revision_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        let Some(accepted) = accepted else {
            return Err(GatewayEdgeError::Unavailable);
        };
        if let Some(issuer) = &self.runtime_authority {
            let issued_at = OffsetDateTime::now_utc();
            let issued = issuer
                .issue_gateway(GatewayRuntimeSessionRequest {
                    invocation_id: capability_domain::GatewayInvocationId::from_uuid(invocation_id),
                    gateway_id: accepted.gateway_id,
                    gateway_revision_id: accepted.gateway_revision_id,
                    issued_at,
                    expires_at: issued_at
                        + time::Duration::try_from(self.session_ttl)
                            .map_err(|_| GatewayEdgeError::Unavailable)?,
                })
                .await;
            if issued.is_err() {
                let _: bool =
                    sqlx::query_scalar("SELECT gateway_invocation_complete($1, 'rejected')")
                        .bind(invocation_id)
                        .fetch_one(&self.pool)
                        .await
                        .map_err(|_| GatewayEdgeError::Unavailable)?;
                return Err(GatewayEdgeError::Unavailable);
            }
        }
        Ok(invocation_id)
    }

    async fn completed(
        &self,
        invocation_id: Uuid,
        outcome: GatewayInvocationOutcome,
    ) -> Result<(), GatewayEdgeError> {
        let completed: bool = sqlx::query_scalar("SELECT gateway_invocation_complete($1, $2)")
            .bind(invocation_id)
            .bind(outcome_name(outcome))
            .fetch_one(&self.pool)
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        if completed {
            Ok(())
        } else {
            Err(GatewayEdgeError::Unavailable)
        }
    }
}

#[derive(sqlx::FromRow)]
struct ActiveRouteRow {
    route_id: Uuid,
    gateway_revision_id: Uuid,
    path: String,
    methods: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct AcceptedInvocationRow {
    gateway_id: Uuid,
    gateway_revision_id: Uuid,
}

fn active_route(
    row: ActiveRouteRow,
    limits: GatewayLimits,
) -> Result<GatewayRouteBinding, GatewayEdgeError> {
    let methods = row
        .methods
        .into_iter()
        .map(|method| persisted_method(&method))
        .collect::<Result<_, _>>()?;
    let path_prefix = row
        .path
        .strip_prefix('/')
        .ok_or(GatewayEdgeError::Unavailable)?
        .to_owned();
    let binding = GatewayRouteBinding {
        route_id: row.route_id,
        gateway_revision_id: row.gateway_revision_id,
        path_prefix,
        methods,
        limits,
    };
    binding.validate()?;
    Ok(binding)
}

fn persisted_method(value: &str) -> Result<Method, GatewayEdgeError> {
    match value {
        "GET" => Ok(Method::GET),
        "POST" => Ok(Method::POST),
        "PUT" => Ok(Method::PUT),
        "PATCH" => Ok(Method::PATCH),
        "DELETE" => Ok(Method::DELETE),
        "HEAD" => Ok(Method::HEAD),
        "OPTIONS" => Ok(Method::OPTIONS),
        _ => Err(GatewayEdgeError::Unavailable),
    }
}

fn canonical_request_path(path_and_query: &str) -> Result<&str, GatewayEdgeError> {
    let path = path_and_query
        .split_once('?')
        .map_or(path_and_query, |(path, _)| path);
    if !path.starts_with('/') || path.contains(['#', '%']) || path.contains("//") {
        return Err(GatewayEdgeError::Contract("ambiguous request path"));
    }
    Ok(path)
}

fn route_matches(route: &GatewayRouteBinding, path: &str) -> bool {
    let public = route.public_path();
    path == public
        || path
            .strip_prefix(&public)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

const fn outcome_name(outcome: GatewayInvocationOutcome) -> &'static str {
    match outcome {
        GatewayInvocationOutcome::Completed => "completed",
        GatewayInvocationOutcome::Failed => "failed",
        GatewayInvocationOutcome::TimedOut => "timed_out",
        GatewayInvocationOutcome::Rejected => "rejected",
    }
}

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
    use std::{collections::BTreeSet, time::Duration};

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

    fn edge_limits() -> GatewayLimits {
        GatewayLimits {
            max_request_body_bytes: 1024,
            max_response_body_bytes: 1024,
            max_request_headers: 16,
            max_response_headers: 16,
            max_path_and_query_bytes: 1024,
            execution_timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn edge_resolution_selects_the_longest_exact_path_segment() {
        let short = GatewayRouteBinding {
            route_id: Uuid::new_v4(),
            gateway_revision_id: Uuid::new_v4(),
            path_prefix: String::from("telegram"),
            methods: BTreeSet::from([Method::POST]),
            limits: edge_limits(),
        };
        let nested = GatewayRouteBinding {
            route_id: Uuid::new_v4(),
            gateway_revision_id: Uuid::new_v4(),
            path_prefix: String::from("telegram/updates"),
            methods: BTreeSet::from([Method::POST]),
            limits: edge_limits(),
        };
        assert!(route_matches(&short, "/gateway/telegram"));
        assert!(route_matches(&nested, "/gateway/telegram/updates"));
        assert!(!route_matches(&short, "/gateway/telegram-bot"));
        assert_eq!(
            canonical_request_path("/gateway/telegram/updates?offset=1").expect("path"),
            "/gateway/telegram/updates"
        );
    }

    #[test]
    fn edge_route_conversion_rejects_unknown_persisted_method() {
        let row = ActiveRouteRow {
            route_id: Uuid::new_v4(),
            gateway_revision_id: Uuid::new_v4(),
            path: String::from("/telegram"),
            methods: vec![String::from("CONNECT")],
        };
        assert!(active_route(row, edge_limits()).is_err());
    }
}
