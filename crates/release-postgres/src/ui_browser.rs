//! Worker-side UI browser handoff issuance adapter.
//!
//! This file contains the worker-side issuance and exchange methods only.
//! The application-role verifier is a separate, read-only method on the same
//! store; worker writes never use the application pool.

use async_trait::async_trait;
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::PostgresMelangeAuthorizer;
use forge_domain::OrganizationId;
use gateway_domain::HttpMethod;
use identity_domain::{BrowserSessionId, RequestId, UserId};
use release_domain::{
    UiInstallationGenerationId, UiInstallationId,
    ui_browser::{UiBrowserHandoffId, UiBrowserRoute, UiBrowserSessionId, child_session_expiry},
};
use release_service::{
    AuthenticateUiBrowserSession, CreateUiBrowserHandoff, CreatedUiBrowserHandoff,
    CreatedUiBrowserSession, ExchangeUiBrowserHandoff, UiBrowserHandoffError,
    UiBrowserRequestRoute, UiBrowserSessionContext, UiBrowserSessionError, UiBrowserSessionStore,
};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

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

    /// Issues one handoff bound to the exact current published generation.
    ///
    /// # Errors
    ///
    /// Returns [`UiBrowserHandoffError::PermissionDenied`] when the actor,
    /// parent, installation, or published release is no longer eligible.
    /// Returns [`UiBrowserHandoffError::InvalidRoute`] for an undeclared route
    /// and [`UiBrowserHandoffError::Unavailable`] for persistence failures.
    // Keep the complete issuance transaction together so its lock order and
    // post-lock authority checks remain auditable.
    #[allow(clippy::too_many_lines)]
    pub async fn create_ui_browser_handoff(
        &self,
        command: CreateUiBrowserHandoff,
    ) -> Result<CreatedUiBrowserHandoff, UiBrowserHandoffError> {
        let mut tx =
            begin_actor_transaction(&self.worker_pool, command.actor_id, command.request_id)
                .await
                .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        let eligible = self
            .lock_and_check_eligibility(
                &mut tx,
                &EligibilityInput {
                    actor_id: command.actor_id,
                    parent_session_id: command.parent_session_id,
                    installation_id: command.installation_id,
                    generation_id: command.generation_id,
                    route: command.route.clone(),
                },
            )
            .await?;
        let handoff_id = UiBrowserHandoffId::new();
        let digest = command.secret.digest().as_bytes();
        let issued = sqlx::query_as::<_, IssuedRow>(
            r"
            INSERT INTO ui_browser_handoffs (
                id, handoff_digest, request_id, actor_id, parent_session_id,
                installation_id, generation_id, organization_id, route,
                issued_at, expires_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9,
                statement_timestamp(), statement_timestamp() + interval '60 seconds'
            ) RETURNING issued_at, expires_at
            ",
        )
        .bind(handoff_id.as_uuid())
        .bind(digest.as_slice())
        .bind(command.request_id.as_uuid())
        .bind(command.actor_id.as_uuid())
        .bind(command.parent_session_id.as_uuid())
        .bind(command.installation_id.as_uuid())
        .bind(command.generation_id.as_uuid())
        .bind(eligible.organization_id)
        .bind(command.route.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        if issued.expires_at != issued.issued_at + Duration::seconds(60) {
            return Err(UiBrowserHandoffError::Unavailable);
        }
        tx.commit()
            .await
            .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        Ok(CreatedUiBrowserHandoff {
            handoff_id,
            actor_id: command.actor_id,
            parent_session_id: command.parent_session_id,
            organization_id: OrganizationId::from_uuid(eligible.organization_id),
            installation_id: command.installation_id,
            generation_id: command.generation_id,
            route: command.route,
            expires_at: issued.expires_at,
        })
    }

    /// Checks mutable authority in canonical lock order and immutable
    /// publication/binding evidence with ordinary worker-readable queries.
    // Keep the shared authority path together so issuance and exchange cannot
    // drift in lock ordering or mutable-state checks.
    #[allow(clippy::too_many_lines)]
    async fn lock_and_check_eligibility(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        input: &EligibilityInput,
    ) -> Result<IssueEligibility, UiBrowserHandoffError> {
        // Match identity-postgres revocation: user first, then that user's
        // session. The session ID never substitutes for actor identity.
        let user_status: Option<String> =
            sqlx::query_scalar(r"SELECT status FROM users WHERE id = $1 FOR UPDATE")
                .bind(input.actor_id.as_uuid())
                .fetch_optional(&mut **tx)
                .await
                .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        if user_status.as_deref() != Some("active") {
            return Err(UiBrowserHandoffError::PermissionDenied);
        }
        let parent = sqlx::query_as::<_, ParentRow>(
            r"
            SELECT user_id, issued_at, expires_at, revoked_at
            FROM human_browser_sessions
            WHERE id = $1 AND user_id = $2
            FOR UPDATE
            ",
        )
        .bind(input.parent_session_id.as_uuid())
        .bind(input.actor_id.as_uuid())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| UiBrowserHandoffError::Unavailable)?
        .ok_or(UiBrowserHandoffError::PermissionDenied)?;

        // Discover the owner before locking it. This follows the installation
        // lifecycle writer's owner-before-installation lock order.
        let discovered = sqlx::query_as::<_, InstallationDiscovery>(
            r"
            SELECT scope, organization_id, project_id, repository_id
            FROM ui_installations WHERE id = $1
            ",
        )
        .bind(input.installation_id.as_uuid())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| UiBrowserHandoffError::Unavailable)?
        .ok_or(UiBrowserHandoffError::PermissionDenied)?;
        let organization_id = lock_owner(tx, &discovered).await?;

        // Re-read the mutable installation pointer after owner locking. The
        // immutable generation is deliberately read without FOR UPDATE.
        let installation = sqlx::query_as::<_, InstallationRow>(
            r"
            SELECT installation.id AS installation_id, installation.scope, installation.lifecycle,
                   installation.organization_id, installation.project_id,
                   installation.repository_id, installation.current_generation_id,
                   generation.id AS generation_id, generation.release_id,
                   generation.ui_key, generation.ui_scope
            FROM ui_installations AS installation
            JOIN ui_installation_generations AS generation
              ON generation.installation_id = installation.id
             AND generation.id = $2
            WHERE installation.id = $1
            FOR UPDATE OF installation
            ",
        )
        .bind(input.installation_id.as_uuid())
        .bind(input.generation_id.as_uuid())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| UiBrowserHandoffError::Unavailable)?
        .ok_or(UiBrowserHandoffError::PermissionDenied)?;
        if installation.lifecycle != "enabled"
            || installation.current_generation_id != installation.generation_id
            || installation.scope != discovered.scope
            || installation.organization_id != discovered.organization_id
            || installation.project_id != discovered.project_id
            || installation.repository_id != discovered.repository_id
        {
            return Err(UiBrowserHandoffError::PermissionDenied);
        }

        let source = sqlx::query_as::<_, SourceRow>(
            r"
            SELECT release_record.state, descriptor.scope, descriptor.route_base,
                   descriptor.content_kind, source_project.id AS source_project_id,
                   source_repository.id AS source_repository_id,
                   source_project.organization_id AS source_org
            FROM releases AS release_record
            JOIN repositories AS source_repository
              ON source_repository.id = release_record.repository_id
            JOIN projects AS source_project
              ON source_project.id = source_repository.project_id
            JOIN release_ui_descriptors AS descriptor
              ON descriptor.release_id = release_record.id
             AND descriptor.ui_key = $2
            WHERE release_record.id = $1
            FOR SHARE OF release_record
            ",
        )
        .bind(installation.release_id)
        .bind(&installation.ui_key)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| UiBrowserHandoffError::Unavailable)?
        .ok_or(UiBrowserHandoffError::PermissionDenied)?;
        if source.state != "published"
            || source.scope != installation.ui_scope
            || source.source_org != organization_id
        {
            return Err(UiBrowserHandoffError::PermissionDenied);
        }
        if input.route.as_str() != source.route_base {
            return Err(UiBrowserHandoffError::InvalidRoute);
        }
        if !matches!(source.content_kind.as_str(), "static" | "managed_service") {
            return Err(UiBrowserHandoffError::InvalidRoute);
        }

        let target = match installation.scope.as_str() {
            "global" => ObjectRef::new(ObjectType::Organization, organization_id),
            "project" => ObjectRef::new(
                ObjectType::Project,
                installation
                    .project_id
                    .ok_or(UiBrowserHandoffError::Unavailable)?,
            ),
            "repository" => ObjectRef::new(
                ObjectType::Repository,
                installation
                    .repository_id
                    .ok_or(UiBrowserHandoffError::Unavailable)?,
            ),
            _ => return Err(UiBrowserHandoffError::Unavailable),
        };
        require_permission(
            &self.authorizer,
            tx,
            input.actor_id,
            Permission::CanRead,
            target,
        )
        .await?;
        if installation.scope == "repository" {
            require_permission(
                &self.authorizer,
                tx,
                input.actor_id,
                Permission::CanRead,
                ObjectRef::new(
                    ObjectType::Project,
                    installation
                        .project_id
                        .ok_or(UiBrowserHandoffError::Unavailable)?,
                ),
            )
            .await?;
        }
        require_permission(
            &self.authorizer,
            tx,
            input.actor_id,
            Permission::CanUse,
            ObjectRef::new(ObjectType::Release, installation.release_id),
        )
        .await?;
        check_bindings(tx, &installation, &source, input.actor_id, &self.authorizer).await?;
        // This final timestamp follows gateway and release-agent locks, so an
        // issuer that waited cannot mint against an expired parent session.
        let now: OffsetDateTime = sqlx::query_scalar(r"SELECT statement_timestamp()")
            .fetch_one(&mut **tx)
            .await
            .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        if parent.user_id != input.actor_id.as_uuid()
            || parent.revoked_at.is_some()
            || parent.issued_at > now
            || parent.expires_at <= now
        {
            return Err(UiBrowserHandoffError::PermissionDenied);
        }

        Ok(IssueEligibility {
            organization_id,
            parent_expires_at: parent.expires_at,
        })
    }
}

async fn begin_actor_transaction(
    pool: &PgPool,
    actor_id: UserId,
    request_id: RequestId,
) -> Result<Transaction<'_, Postgres>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        r"SELECT set_config('hephaestus.actor_id', $1, true),
                  set_config('hephaestus.subject_type', 'user', true),
                  set_config('hephaestus.request_id', $2, true)",
    )
    .bind(actor_id.as_uuid().to_string())
    .bind(request_id.as_uuid().to_string())
    .execute(&mut *tx)
    .await?;
    Ok(tx)
}

async fn require_permission(
    authorizer: &PostgresMelangeAuthorizer,
    tx: &mut Transaction<'_, Postgres>,
    actor: UserId,
    permission: Permission,
    object: ObjectRef,
) -> Result<(), UiBrowserHandoffError> {
    let decision = authorizer
        .check(tx, Subject::User(actor), permission, object)
        .await
        .map_err(|_| UiBrowserHandoffError::Unavailable)?;
    if decision == AuthorizationDecision::Allow {
        Ok(())
    } else {
        Err(UiBrowserHandoffError::PermissionDenied)
    }
}

async fn lock_owner(
    tx: &mut Transaction<'_, Postgres>,
    discovered: &InstallationDiscovery,
) -> Result<Uuid, UiBrowserHandoffError> {
    match discovered.scope.as_str() {
        "global" => {
            sqlx::query_scalar::<_, Uuid>(r"SELECT id FROM organizations WHERE id = $1 FOR UPDATE")
                .bind(
                    discovered
                        .organization_id
                        .ok_or(UiBrowserHandoffError::Unavailable)?,
                )
                .fetch_optional(&mut **tx)
                .await
                .map_err(|_| UiBrowserHandoffError::Unavailable)?
                .ok_or(UiBrowserHandoffError::PermissionDenied)
        }
        "project" => sqlx::query_scalar::<_, Uuid>(
            r"SELECT organization_id FROM projects WHERE id = $1 FOR UPDATE",
        )
        .bind(
            discovered
                .project_id
                .ok_or(UiBrowserHandoffError::Unavailable)?,
        )
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| UiBrowserHandoffError::Unavailable)?
        .ok_or(UiBrowserHandoffError::PermissionDenied),
        "repository" => sqlx::query_scalar::<_, Uuid>(
            r"
            SELECT project.organization_id
            FROM repositories AS repository
            JOIN projects AS project ON project.id = repository.project_id
            WHERE repository.id = $1
            FOR UPDATE OF repository, project
            ",
        )
        .bind(
            discovered
                .repository_id
                .ok_or(UiBrowserHandoffError::Unavailable)?,
        )
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| UiBrowserHandoffError::Unavailable)?
        .ok_or(UiBrowserHandoffError::PermissionDenied),
        _ => Err(UiBrowserHandoffError::Unavailable),
    }
}

// Keep gateway binding validation together so each release declaration and
// its persisted binding are checked under one auditable lock sequence.
#[allow(clippy::too_many_lines)]
async fn check_bindings(
    tx: &mut Transaction<'_, Postgres>,
    installation: &InstallationRow,
    source: &SourceRow,
    actor: UserId,
    authorizer: &PostgresMelangeAuthorizer,
) -> Result<(), UiBrowserHandoffError> {
    let managed = sqlx::query_as::<_, ManagedRow>(
        r"SELECT gateway_name, route, release_agent_id
           FROM release_ui_managed_services
           WHERE release_id = $1 AND ui_key = $2",
    )
    .bind(installation.release_id)
    .bind(&installation.ui_key)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| UiBrowserHandoffError::Unavailable)?;
    let api = sqlx::query_as::<_, ApiRow>(
        r"SELECT api_key, gateway_name, method, route, release_agent_id
           FROM release_ui_api_bindings WHERE release_id = $1 AND ui_key = $2",
    )
    .bind(installation.release_id)
    .bind(&installation.ui_key)
    .fetch_all(&mut **tx)
    .await
    .map_err(|_| UiBrowserHandoffError::Unavailable)?;
    let declared_api: BTreeMap<_, _> = api.iter().map(|row| (row.api_key.clone(), row)).collect();
    let bindings = sqlx::query_as::<_, BindingRow>(
        r"SELECT binding_kind, binding_key, release_id, ui_key, method, route,
                  release_agent_id, gateway_id, gateway_revision_id,
                  gateway_name, exposure
           FROM ui_installation_bindings
           WHERE installation_id = $1 AND generation_id = $2",
    )
    .bind(installation.installation_id)
    .bind(installation.generation_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|_| UiBrowserHandoffError::Unavailable)?;
    if managed.is_some() || !api.is_empty() || !bindings.is_empty() {
        require_permission(
            authorizer,
            tx,
            actor,
            Permission::CanRead,
            ObjectRef::new(ObjectType::Project, source.source_project_id),
        )
        .await?;
    }
    let mut seen_api = BTreeSet::new();
    let mut seen_managed = false;
    for binding in bindings {
        if binding.release_id != installation.release_id || binding.ui_key != installation.ui_key {
            return Err(UiBrowserHandoffError::PermissionDenied);
        }
        let gateway = sqlx::query_as::<_, GatewayRow>(
            r"
            SELECT gateway.name AS gateway_name, gateway.lifecycle,
                   gateway.active_revision_id, revision.id AS revision_id,
                   revision.release_id, revision.release_agent_id,
                   revision.handler_contract,
                   revision.project_id, revision.repository_id,
                   revision.exposure, route.path, route.enabled, route.methods
            FROM gateways AS gateway
            JOIN gateway_revisions AS revision
              ON revision.id = $1 AND revision.gateway_id = gateway.id
            JOIN gateway_routes AS route
              ON route.gateway_revision_id = revision.id
             AND route.gateway_id = gateway.id
            WHERE gateway.id = $2
            FOR SHARE OF gateway
            ",
        )
        .bind(binding.gateway_revision_id)
        .bind(binding.gateway_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(|_| UiBrowserHandoffError::Unavailable)?
        .into_iter()
        .find(|candidate| {
            candidate.enabled
                && route_covers(&candidate.path, &binding.route)
                && candidate
                    .methods
                    .iter()
                    .any(|method| method == &binding.method)
        })
        .ok_or(UiBrowserHandoffError::PermissionDenied)?;
        if gateway.lifecycle != "enabled"
            || gateway.active_revision_id != Some(gateway.revision_id)
            || gateway.revision_id != binding.gateway_revision_id
            || gateway.release_id != installation.release_id
            || gateway.release_agent_id != binding.release_agent_id
            || gateway.project_id != source.source_project_id
            || gateway.repository_id != source.source_repository_id
            || gateway.exposure != binding.exposure
            || gateway.gateway_name != binding.gateway_name
        {
            return Err(UiBrowserHandoffError::PermissionDenied);
        }
        require_permission(
            authorizer,
            tx,
            actor,
            Permission::CanRead,
            ObjectRef::new(ObjectType::Gateway, binding.gateway_id),
        )
        .await?;
        match binding.binding_kind.as_str() {
            "managed_service" if binding.binding_key == "service" && binding.method == "GET" => {
                if gateway.handler_contract != "http.service.v1" {
                    return Err(UiBrowserHandoffError::PermissionDenied);
                }
                let Some(row) = managed.as_ref() else {
                    return Err(UiBrowserHandoffError::PermissionDenied);
                };
                if row.gateway_name != binding.gateway_name
                    || row.route != binding.route
                    || row.release_agent_id != binding.release_agent_id
                {
                    return Err(UiBrowserHandoffError::PermissionDenied);
                }
                seen_managed = true;
                require_permission(
                    authorizer,
                    tx,
                    actor,
                    Permission::CanUse,
                    ObjectRef::new(ObjectType::ReleaseAgent, binding.release_agent_id),
                )
                .await?;
            }
            "api" => {
                let Some(row) = declared_api.get(&binding.binding_key) else {
                    return Err(UiBrowserHandoffError::PermissionDenied);
                };
                if row.gateway_name != binding.gateway_name
                    || row.method != binding.method
                    || row.route != binding.route
                    || row.release_agent_id != binding.release_agent_id
                {
                    return Err(UiBrowserHandoffError::PermissionDenied);
                }
                seen_api.insert(binding.binding_key);
                require_permission(
                    authorizer,
                    tx,
                    actor,
                    Permission::CanUse,
                    ObjectRef::new(ObjectType::ReleaseAgent, binding.release_agent_id),
                )
                .await?;
            }
            _ => return Err(UiBrowserHandoffError::PermissionDenied),
        }
    }
    if (source.content_kind == "managed_service") != seen_managed
        || managed.is_some() != seen_managed
        || seen_api.len() != declared_api.len()
    {
        return Err(UiBrowserHandoffError::PermissionDenied);
    }
    Ok(())
}

fn route_covers(declared: &str, selected: &str) -> bool {
    selected == declared
        || selected
            .strip_prefix(declared)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

#[derive(Debug)]
struct EligibilityInput {
    actor_id: UserId,
    parent_session_id: identity_domain::BrowserSessionId,
    installation_id: UiInstallationId,
    generation_id: release_domain::UiInstallationGenerationId,
    route: UiBrowserRoute,
}

#[derive(Debug)]
struct IssueEligibility {
    organization_id: Uuid,
    parent_expires_at: OffsetDateTime,
}

#[derive(Debug, FromRow)]
struct ParentRow {
    user_id: Uuid,
    issued_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    revoked_at: Option<OffsetDateTime>,
}

#[derive(Debug, FromRow)]
struct InstallationDiscovery {
    scope: String,
    organization_id: Option<Uuid>,
    project_id: Option<Uuid>,
    repository_id: Option<Uuid>,
}

#[derive(Debug, FromRow)]
struct InstallationRow {
    installation_id: Uuid,
    scope: String,
    lifecycle: String,
    organization_id: Option<Uuid>,
    project_id: Option<Uuid>,
    repository_id: Option<Uuid>,
    current_generation_id: Uuid,
    generation_id: Uuid,
    release_id: Uuid,
    ui_key: String,
    ui_scope: String,
}

#[derive(Debug, FromRow)]
struct SourceRow {
    state: String,
    scope: String,
    route_base: String,
    content_kind: String,
    source_project_id: Uuid,
    source_repository_id: Uuid,
    source_org: Uuid,
}

#[derive(Debug, FromRow)]
struct BindingRow {
    binding_kind: String,
    binding_key: String,
    release_id: Uuid,
    ui_key: String,
    method: String,
    route: String,
    release_agent_id: Uuid,
    gateway_id: Uuid,
    gateway_revision_id: Uuid,
    gateway_name: String,
    exposure: String,
}

#[derive(Debug, FromRow)]
struct GatewayRow {
    gateway_name: String,
    lifecycle: String,
    active_revision_id: Option<Uuid>,
    revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    handler_contract: String,
    project_id: Uuid,
    repository_id: Uuid,
    exposure: String,
    path: String,
    enabled: bool,
    methods: Vec<String>,
}

#[derive(Debug, FromRow)]
struct ManagedRow {
    gateway_name: String,
    route: String,
    release_agent_id: Uuid,
}

#[derive(Debug, FromRow)]
struct ApiRow {
    api_key: String,
    gateway_name: String,
    method: String,
    route: String,
    release_agent_id: Uuid,
}

#[derive(Debug, FromRow)]
struct IssuedRow {
    issued_at: OffsetDateTime,
    expires_at: OffsetDateTime,
}

impl PgUiBrowserSessionStore {
    /// Atomically exchanges one handoff for one child session.
    ///
    /// This is called by the trusted UI-origin handler after it resolves the
    /// exact generation host behind Caddy's namespace forward. Phoenix creates
    /// the handoff secret through its sensitive authenticated RPC boundary;
    /// this method never returns a secret, digest, or cookie value.
    // Keep child insertion and one-time consumption in one auditable
    // transaction after every authority lock and the final timestamp read.
    #[allow(clippy::too_many_lines)]
    ///
    /// # Errors
    ///
    /// Returns [`UiBrowserHandoffError::InvalidOrExpired`] when the handoff is
    /// consumed, expired, bound to another generation, or no longer eligible.
    /// Returns [`UiBrowserHandoffError::Unavailable`] for persistence failures.
    pub async fn exchange_ui_browser_handoff(
        &self,
        command: ExchangeUiBrowserHandoff,
    ) -> Result<CreatedUiBrowserSession, UiBrowserHandoffError> {
        let mut tx = begin_exchange_transaction(&self.worker_pool, command.request_id)
            .await
            .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        let handoff_digest = command.handoff_secret.digest().as_bytes();
        let handoff = sqlx::query_as::<_, HandoffRow>(
            r"
            SELECT id, actor_id, parent_session_id, installation_id,
                   generation_id, organization_id, route, issued_at, expires_at,
                   consumed_at
            FROM ui_browser_handoffs
            WHERE handoff_digest = $1
            FOR UPDATE
            ",
        )
        .bind(handoff_digest.as_slice())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| UiBrowserHandoffError::Unavailable)?
        .ok_or(UiBrowserHandoffError::InvalidOrExpired)?;
        if handoff.consumed_at.is_some()
            || handoff.generation_id != command.expected_generation_id.as_uuid()
        {
            return Err(UiBrowserHandoffError::InvalidOrExpired);
        }
        set_exchange_actor_context(&mut tx, handoff.actor_id, command.request_id)
            .await
            .map_err(|_| UiBrowserHandoffError::Unavailable)?;

        let route = UiBrowserRoute::parse(&handoff.route)
            .map_err(|_| UiBrowserHandoffError::InvalidOrExpired)?;
        // Reuse the shared eligibility path to preserve user -> parent ->
        // owner -> installation -> source ordering and every current check.
        let eligibility = self
            .lock_and_check_eligibility(
                &mut tx,
                &EligibilityInput {
                    actor_id: UserId::from_uuid(handoff.actor_id),
                    parent_session_id: identity_domain::BrowserSessionId::from_uuid(
                        handoff.parent_session_id,
                    ),
                    installation_id: UiInstallationId::from_uuid(handoff.installation_id),
                    generation_id: release_domain::UiInstallationGenerationId::from_uuid(
                        handoff.generation_id,
                    ),
                    route: route.clone(),
                },
            )
            .await
            .map_err(|error| match error {
                UiBrowserHandoffError::Unavailable => error,
                _ => UiBrowserHandoffError::InvalidOrExpired,
            })?;
        let parent_survives_handoff = eligibility.parent_expires_at > handoff.issued_at;
        if eligibility.organization_id != handoff.organization_id || !parent_survives_handoff {
            return Err(UiBrowserHandoffError::InvalidOrExpired);
        }

        // This clock is after the handoff, parent, owner, installation, and
        // mutable release locks. It closes the wait/expiry race before issue.
        let issue_now: OffsetDateTime = sqlx::query_scalar(r"SELECT statement_timestamp()")
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        if handoff.issued_at > issue_now || handoff.expires_at <= issue_now {
            return Err(UiBrowserHandoffError::InvalidOrExpired);
        }
        let child_expires_at = child_session_expiry(issue_now, eligibility.parent_expires_at)
            .map_err(|_| UiBrowserHandoffError::InvalidOrExpired)?;
        let session_id = UiBrowserSessionId::new();
        let session_digest = command.child_secret.digest().as_bytes();
        let inserted = sqlx::query_as::<_, ChildRow>(
            r"
            INSERT INTO ui_browser_sessions (
                id, session_digest, request_id, handoff_id, parent_session_id,
                installation_id, generation_id, organization_id, route,
                issued_at, expires_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11
            ) RETURNING issued_at, expires_at
            ",
        )
        .bind(session_id.as_uuid())
        .bind(session_digest.as_slice())
        .bind(command.request_id.as_uuid())
        .bind(handoff.id)
        .bind(handoff.parent_session_id)
        .bind(handoff.installation_id)
        .bind(handoff.generation_id)
        .bind(handoff.organization_id)
        .bind(&handoff.route)
        .bind(issue_now)
        .bind(child_expires_at)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        if inserted.issued_at != issue_now || inserted.expires_at != child_expires_at {
            return Err(UiBrowserHandoffError::Unavailable);
        }
        let consumed = sqlx::query(
            r"UPDATE ui_browser_handoffs
               SET consumed_at = $2
               WHERE id = $1 AND consumed_at IS NULL",
        )
        .bind(handoff.id)
        .bind(issue_now)
        .execute(&mut *tx)
        .await
        .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        if consumed.rows_affected() != 1 {
            return Err(UiBrowserHandoffError::InvalidOrExpired);
        }
        tx.commit()
            .await
            .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        Ok(CreatedUiBrowserSession {
            context: UiBrowserSessionContext {
                session_id,
                parent_session_id: identity_domain::BrowserSessionId::from_uuid(
                    handoff.parent_session_id,
                ),
                actor_id: UserId::from_uuid(handoff.actor_id),
                organization_id: OrganizationId::from_uuid(handoff.organization_id),
                installation_id: UiInstallationId::from_uuid(handoff.installation_id),
                generation_id: release_domain::UiInstallationGenerationId::from_uuid(
                    handoff.generation_id,
                ),
                route,
                expires_at: inserted.expires_at,
            },
        })
    }
}

impl PgUiBrowserSessionStore {
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

async fn begin_exchange_transaction(
    pool: &PgPool,
    request_id: RequestId,
) -> Result<Transaction<'_, Postgres>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        r"SELECT set_config('hephaestus.subject_type', 'user', true),
                  set_config('hephaestus.request_id', $1, true)",
    )
    .bind(request_id.as_uuid().to_string())
    .execute(&mut *tx)
    .await?;
    Ok(tx)
}

async fn set_exchange_actor_context(
    tx: &mut Transaction<'_, Postgres>,
    actor_id: Uuid,
    request_id: RequestId,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r"SELECT set_config('hephaestus.actor_id', $1, true),
                  set_config('hephaestus.subject_type', 'user', true),
                  set_config('hephaestus.request_id', $2, true)",
    )
    .bind(actor_id.to_string())
    .bind(request_id.as_uuid().to_string())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[derive(Debug, FromRow)]
struct HandoffRow {
    id: Uuid,
    actor_id: Uuid,
    parent_session_id: Uuid,
    installation_id: Uuid,
    generation_id: Uuid,
    organization_id: Uuid,
    route: String,
    issued_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    consumed_at: Option<OffsetDateTime>,
}

#[derive(Debug, FromRow)]
struct ChildRow {
    issued_at: OffsetDateTime,
    expires_at: OffsetDateTime,
}
