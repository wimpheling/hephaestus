use super::PgUiBrowserSessionStore;
use super::authorization::{check_bindings, lock_owner, require_permission};
use super::rows::{
    EligibilityInput, InstallationDiscovery, InstallationRow, IssueEligibility, ParentRow,
    SourceRow,
};
use authz_domain::{ObjectRef, ObjectType, Permission};
use release_service::UiBrowserHandoffError;
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;

impl PgUiBrowserSessionStore {
    /// Checks mutable authority in canonical lock order and immutable
    /// publication/binding evidence with ordinary worker-readable queries.
    // Keep the shared authority path together so issuance and exchange cannot
    // drift in lock ordering or mutable-state checks.
    #[allow(clippy::too_many_lines)]
    pub(super) async fn lock_and_check_eligibility(
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
                   descriptor.presentation, descriptor.content_kind,
                   source_project.id AS source_project_id,
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
            presentation: source.presentation,
        })
    }
}
