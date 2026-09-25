//! Application-role read projection for installed UI navigation.

use authz_postgres::begin_actor_transaction;
use forge_domain::{OrganizationId, ProjectId, RepositoryId};
use identity_domain::AuthenticatedIdentity;
use release_domain::{
    ReleaseId, UiInstallationGenerationId, UiInstallationId, UiInstallationState,
    UiInstallationTarget,
    ui::{UiIcon, UiKey, UiLabel, UiPresentation, UiRoutePath},
};
use release_service::{
    ListUiInstallations, MAX_UI_INSTALLATION_PAGE_SIZE, UiInstallationContentKind,
    UiInstallationNavigation, UiInstallationNavigationError, UiInstallationNavigationPage,
    UiInstallationNavigator,
};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

/// Application-role `PostgreSQL` implementation of installed UI navigation.
#[derive(Clone)]
pub struct PgUiInstallationNavigator {
    pool: PgPool,
}

impl PgUiInstallationNavigator {
    /// Creates a navigator over an application-role capable pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, FromRow)]
struct NavigationRow {
    installation_id: Uuid,
    generation_id: Uuid,
    organization_id: Uuid,
    project_id: Option<Uuid>,
    repository_id: Option<Uuid>,
    scope: String,
    lifecycle: String,
    release_id: Uuid,
    ui_key: String,
    label: String,
    icon: String,
    presentation: String,
    route_base: String,
    content_kind: String,
    launchable: bool,
}

#[async_trait::async_trait]
impl UiInstallationNavigator for PgUiInstallationNavigator {
    // Keep the authorization cursor query, projection query, and transaction
    // boundary together so navigation remains auditable as one read path.
    #[allow(clippy::too_many_lines)]
    async fn list_ui_installations(
        &self,
        identity: &AuthenticatedIdentity,
        request: ListUiInstallations,
    ) -> Result<UiInstallationNavigationPage, UiInstallationNavigationError> {
        if !(1..=MAX_UI_INSTALLATION_PAGE_SIZE).contains(&request.page.size) {
            return Err(UiInstallationNavigationError::InvalidPage);
        }
        let (scope, target_id) = request.target.scope_and_id();
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(|_| UiInstallationNavigationError::Unavailable)?;
        let target_is_in_organization: bool = match scope {
            "global" => {
                sqlx::query_scalar(
                    "SELECT EXISTS (
                     SELECT 1 FROM organizations
                     WHERE id = $1
                )",
                )
                .bind(request.organization_id.as_uuid())
                .fetch_one(&mut *transaction)
                .await
            }
            "project" => {
                sqlx::query_scalar(
                    "SELECT EXISTS (
                     SELECT 1 FROM projects
                     WHERE id = $2 AND organization_id = $1
                )",
                )
                .bind(request.organization_id.as_uuid())
                .bind(target_id)
                .fetch_one(&mut *transaction)
                .await
            }
            "repository" => {
                sqlx::query_scalar(
                    "SELECT EXISTS (
                     SELECT 1
                     FROM repositories AS target_repository
                     JOIN projects AS target_project
                       ON target_project.id = target_repository.project_id
                     WHERE target_repository.id = $2
                       AND target_project.organization_id = $1
                )",
                )
                .bind(request.organization_id.as_uuid())
                .bind(target_id)
                .fetch_one(&mut *transaction)
                .await
            }
            _ => return Err(UiInstallationNavigationError::InvalidPage),
        }
        .map_err(|_| UiInstallationNavigationError::Unavailable)?;
        if !target_is_in_organization {
            return Err(UiInstallationNavigationError::InvalidPage);
        }
        if let Some(after) = request.page.after {
            let cursor_is_in_scope: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                     SELECT 1
                     FROM ui_installations AS cursor
                     LEFT JOIN repositories AS cursor_repository
                       ON cursor_repository.id = cursor.repository_id
                     LEFT JOIN projects AS cursor_project
                       ON cursor_project.id = COALESCE(cursor_repository.project_id,
                                                       cursor.project_id)
                     WHERE cursor.id = $4
                       AND cursor.lifecycle <> 'removed'
                       AND (
                            ($2 = 'global'
                             AND cursor.scope = 'global'
                             AND cursor.organization_id = $1)
                         OR ($2 = 'project'
                             AND cursor.scope = 'project'
                             AND cursor.project_id = $3
                             AND cursor_project.organization_id = $1)
                         OR ($2 = 'repository'
                             AND cursor.scope = 'repository'
                             AND cursor.repository_id = $3
                             AND cursor_project.organization_id = $1)
                       )
                )",
            )
            .bind(request.organization_id.as_uuid())
            .bind(scope)
            .bind(target_id)
            .bind(after.as_uuid())
            .fetch_one(&mut *transaction)
            .await
            .map_err(|_| UiInstallationNavigationError::Unavailable)?;
            if !cursor_is_in_scope {
                return Err(UiInstallationNavigationError::InvalidPage);
            }
        }
        let rows = sqlx::query_as::<_, NavigationRow>(
            "SELECT installation.id AS installation_id,
                    generation.id AS generation_id,
                    COALESCE(installation.organization_id,
                             target_project.organization_id) AS organization_id,
                    installation.project_id,
                    installation.repository_id,
                    installation.scope,
                    installation.lifecycle,
                    generation.release_id,
                    generation.ui_key,
                    descriptor.label,
                    descriptor.icon,
                    descriptor.presentation,
                    descriptor.route_base,
                    descriptor.content_kind,
                    -- Descriptor RLS supplies source CanRead; this separate
                    -- predicate is only the current launchability hint.
                    (
                        installation.lifecycle = 'enabled'
                        AND check_permission('user', hephaestus_actor_id(), 'can_use',
                            'release', generation.release_id::text) = 1
                        AND NOT EXISTS (
                            SELECT 1
                            FROM ui_installation_bindings binding
                            WHERE binding.generation_id = generation.id
                              AND check_permission('user', hephaestus_actor_id(), 'can_use',
                                  'release_agent', binding.release_agent_id::text) <> 1
                        )
                    ) AS launchable
             FROM ui_installations AS installation
             JOIN ui_installation_generations AS generation
               ON generation.id = installation.current_generation_id
              AND generation.installation_id = installation.id
             JOIN release_ui_descriptors AS descriptor
               ON descriptor.release_id = generation.release_id
              AND descriptor.ui_key = generation.ui_key
              AND descriptor.scope = generation.ui_scope
             LEFT JOIN repositories AS target_repository
               ON target_repository.id = installation.repository_id
             LEFT JOIN projects AS target_project
               ON target_project.id = COALESCE(target_repository.project_id,
                                               installation.project_id)
             WHERE installation.lifecycle <> 'removed'
               AND (
                    ($2 = 'global'
                     AND installation.scope = 'global'
                     AND installation.organization_id = $1)
                 OR ($2 = 'project'
                     AND installation.scope = 'project'
                     AND installation.project_id = $3
                     AND target_project.organization_id = $1)
                 OR ($2 = 'repository'
                     AND installation.scope = 'repository'
                     AND installation.repository_id = $3
                     AND target_project.organization_id = $1)
               )
               AND ($4::uuid IS NULL OR (installation.created_at, installation.id) <
                    (SELECT cursor.created_at, cursor.id
                     FROM ui_installations AS cursor
                     WHERE cursor.id = $4))
             ORDER BY installation.created_at DESC, installation.id DESC
             LIMIT $5",
        )
        .bind(request.organization_id.as_uuid())
        .bind(scope)
        .bind(target_id)
        .bind(request.page.after.map(UiInstallationId::as_uuid))
        .bind(request.page.size + 1)
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| UiInstallationNavigationError::Unavailable)?;
        transaction
            .commit()
            .await
            .map_err(|_| UiInstallationNavigationError::Unavailable)?;

        let limit = usize::try_from(request.page.size)
            .map_err(|_| UiInstallationNavigationError::InvalidPage)?;
        let has_more = rows.len() > limit;
        let projections = rows
            .into_iter()
            .take(limit)
            .map(NavigationRow::try_into_projection)
            .collect::<Result<Vec<_>, _>>()?;
        let next = has_more
            .then(|| projections.last().map(|row| row.installation_id))
            .flatten();
        Ok(UiInstallationNavigationPage {
            installations: projections,
            next,
        })
    }
}

impl NavigationRow {
    fn try_into_projection(
        self,
    ) -> Result<UiInstallationNavigation, UiInstallationNavigationError> {
        let organization_id = OrganizationId::from_uuid(self.organization_id);
        let target = match self.scope.as_str() {
            "global" if self.project_id.is_none() && self.repository_id.is_none() => {
                UiInstallationTarget::organization(organization_id)
            }
            "project" if self.project_id.is_some() && self.repository_id.is_none() => {
                UiInstallationTarget::project(ProjectId::from_uuid(
                    self.project_id
                        .ok_or(UiInstallationNavigationError::InvalidStoredData)?,
                ))
            }
            "repository" if self.repository_id.is_some() => {
                UiInstallationTarget::repository(RepositoryId::from_uuid(
                    self.repository_id
                        .ok_or(UiInstallationNavigationError::InvalidStoredData)?,
                ))
            }
            _ => return Err(UiInstallationNavigationError::InvalidStoredData),
        };
        let lifecycle = match self.lifecycle.as_str() {
            "enabled" => UiInstallationState::Enabled,
            "disabled" => UiInstallationState::Disabled,
            "removed" => UiInstallationState::Removed,
            _ => return Err(UiInstallationNavigationError::InvalidStoredData),
        };
        let icon = match self.icon.as_str() {
            "app" => UiIcon::App,
            "chat" => UiIcon::Chat,
            "code" => UiIcon::Code,
            "book" => UiIcon::Book,
            "chart" => UiIcon::Chart,
            _ => return Err(UiInstallationNavigationError::InvalidStoredData),
        };
        let presentation = match self.presentation.as_str() {
            "iframe" => UiPresentation::Iframe,
            "full_page" => UiPresentation::FullPage,
            _ => return Err(UiInstallationNavigationError::InvalidStoredData),
        };
        let content_kind = match self.content_kind.as_str() {
            "static" => UiInstallationContentKind::Static,
            "managed_service" => UiInstallationContentKind::ManagedService,
            _ => return Err(UiInstallationNavigationError::InvalidStoredData),
        };
        Ok(UiInstallationNavigation {
            installation_id: UiInstallationId::from_uuid(self.installation_id),
            generation_id: UiInstallationGenerationId::from_uuid(self.generation_id),
            organization_id,
            target,
            lifecycle,
            release_id: ReleaseId::from_uuid(self.release_id),
            ui_key: UiKey::parse(self.ui_key)
                .map_err(|_| UiInstallationNavigationError::InvalidStoredData)?,
            label: UiLabel::parse(self.label)
                .map_err(|_| UiInstallationNavigationError::InvalidStoredData)?,
            icon,
            presentation,
            route_base: UiRoutePath::parse(self.route_base)
                .map_err(|_| UiInstallationNavigationError::InvalidStoredData)?,
            content_kind,
            launchable: self.launchable,
        })
    }
}
