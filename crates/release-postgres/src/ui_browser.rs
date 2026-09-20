//! Worker-side UI browser handoff issuance adapter.
//!
//! This slice creates one-time handoffs. Exchange, child-session validation,
//! and event APIs remain separate boundaries.

use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::PostgresMelangeAuthorizer;
use forge_domain::OrganizationId;
use identity_domain::{RequestId, UserId};
use release_domain::ui_browser::UiBrowserHandoffId;
use release_service::{CreateUiBrowserHandoff, CreatedUiBrowserHandoff, UiBrowserHandoffError};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

/// `PostgreSQL` store for trusted UI browser handoff issuance.
pub struct PgUiBrowserSessionStore {
    worker_pool: PgPool,
    _app_pool: PgPool,
    authorizer: PostgresMelangeAuthorizer,
}

impl PgUiBrowserSessionStore {
    /// Creates a store with separate worker and application pools.
    #[must_use]
    pub const fn new(worker_pool: PgPool, app_pool: PgPool) -> Self {
        Self {
            worker_pool,
            _app_pool: app_pool,
            authorizer: PostgresMelangeAuthorizer,
        }
    }

    /// Issues one handoff bound to the exact current published generation.
    ///
    /// # Errors
    ///
    /// Returns a redacted failure when the actor is not authorized, the
    /// installation or generation is unavailable, or persistence fails.
    // Keep the full lock, publication, binding, and authorization transaction
    // together so its ordering remains auditable at this boundary.
    #[allow(clippy::too_many_lines)]
    pub async fn create_ui_browser_handoff(
        &self,
        command: CreateUiBrowserHandoff,
    ) -> Result<CreatedUiBrowserHandoff, UiBrowserHandoffError> {
        let mut tx =
            begin_actor_transaction(&self.worker_pool, command.actor_id, command.request_id)
                .await
                .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        let eligible = self.lock_and_check_eligibility(&mut tx, &command).await?;
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
    // The ordered checks are intentionally kept in one transaction to prove
    // the post-lock expiry and authority revalidation boundary.
    #[allow(clippy::too_many_lines)]
    async fn lock_and_check_eligibility(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        command: &CreateUiBrowserHandoff,
    ) -> Result<IssueEligibility, UiBrowserHandoffError> {
        // Match identity-postgres revocation: user first, then that user's
        // session. The session ID never substitutes for actor identity.
        let user_status: Option<String> =
            sqlx::query_scalar(r"SELECT status FROM users WHERE id = $1 FOR UPDATE")
                .bind(command.actor_id.as_uuid())
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
        .bind(command.parent_session_id.as_uuid())
        .bind(command.actor_id.as_uuid())
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
        .bind(command.installation_id.as_uuid())
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
        .bind(command.installation_id.as_uuid())
        .bind(command.generation_id.as_uuid())
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
                   descriptor.content_kind, source_project.organization_id AS source_org
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
        // Capture the database instant only after every potentially waiting
        // lock, including the mutable release state lock above.
        let now: OffsetDateTime = sqlx::query_scalar(r"SELECT statement_timestamp()")
            .fetch_one(&mut **tx)
            .await
            .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        if parent.user_id != command.actor_id.as_uuid()
            || parent.revoked_at.is_some()
            || parent.issued_at > now
            || parent.expires_at <= now
        {
            return Err(UiBrowserHandoffError::PermissionDenied);
        }
        if source.state != "published"
            || source.scope != installation.ui_scope
            || source.source_org != organization_id
        {
            return Err(UiBrowserHandoffError::PermissionDenied);
        }
        if command.route.as_str() != source.route_base {
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
            command.actor_id,
            Permission::CanRead,
            target,
        )
        .await?;
        if installation.scope == "repository" {
            require_permission(
                &self.authorizer,
                tx,
                command.actor_id,
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
            command.actor_id,
            Permission::CanUse,
            ObjectRef::new(ObjectType::Release, installation.release_id),
        )
        .await?;
        check_bindings(
            tx,
            &installation,
            &source.content_kind,
            command.actor_id,
            &self.authorizer,
        )
        .await?;

        Ok(IssueEligibility { organization_id })
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

async fn check_bindings(
    tx: &mut Transaction<'_, Postgres>,
    installation: &InstallationRow,
    content_kind: &str,
    actor: UserId,
    authorizer: &PostgresMelangeAuthorizer,
) -> Result<(), UiBrowserHandoffError> {
    let managed = sqlx::query_as::<_, ManagedRow>(
        r"SELECT route, release_agent_id FROM release_ui_managed_services
           WHERE release_id = $1 AND ui_key = $2",
    )
    .bind(installation.release_id)
    .bind(&installation.ui_key)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| UiBrowserHandoffError::Unavailable)?;
    let api = sqlx::query_as::<_, ApiRow>(
        r"SELECT api_key, method, route, release_agent_id
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
                  release_agent_id
           FROM ui_installation_bindings
           WHERE installation_id = $1 AND generation_id = $2",
    )
    .bind(installation.installation_id)
    .bind(installation.generation_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|_| UiBrowserHandoffError::Unavailable)?;
    let mut seen_api = BTreeSet::new();
    let mut seen_managed = false;
    for binding in bindings {
        if binding.release_id != installation.release_id || binding.ui_key != installation.ui_key {
            return Err(UiBrowserHandoffError::PermissionDenied);
        }
        match binding.binding_kind.as_str() {
            "managed_service" if binding.binding_key == "service" && binding.method == "GET" => {
                let Some(row) = managed.as_ref() else {
                    return Err(UiBrowserHandoffError::PermissionDenied);
                };
                if row.route != binding.route || row.release_agent_id != binding.release_agent_id {
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
                if row.method != binding.method
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
    if (content_kind == "managed_service") != seen_managed
        || managed.is_some() != seen_managed
        || seen_api.len() != declared_api.len()
    {
        return Err(UiBrowserHandoffError::PermissionDenied);
    }
    Ok(())
}

#[derive(Debug)]
struct IssueEligibility {
    organization_id: Uuid,
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
}

#[derive(Debug, FromRow)]
struct ManagedRow {
    route: String,
    release_agent_id: Uuid,
}

#[derive(Debug, FromRow)]
struct ApiRow {
    api_key: String,
    method: String,
    route: String,
    release_agent_id: Uuid,
}

#[derive(Debug, FromRow)]
struct IssuedRow {
    issued_at: OffsetDateTime,
    expires_at: OffsetDateTime,
}
