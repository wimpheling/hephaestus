use super::persistence::map_authorization_public;
use super::{InstallationOwnerRow, LockedInstallationRow, OwnerRow};
use crate::ReleaseService;
use authz_domain::{ObjectRef, ObjectType, Permission};
use forge_domain::{OrganizationId, ProjectId, RepositoryId};
use identity_domain::AuthenticatedIdentity;
use release_domain::{UiInstallationId, UiInstallationState, UiInstallationTarget};
use release_service::{UiInstallationError, UiInstallationReceiptScope};
use sqlx::{Postgres, Transaction};

pub(super) async fn load_installation_owner(
    tx: &mut Transaction<'_, Postgres>,
    installation_id: UiInstallationId,
) -> Result<Option<InstallationOwnerRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT installation.project_id,
                COALESCE(installation.organization_id, project.organization_id)
                    AS organization_id,
                installation.repository_id, installation.scope, installation.ui_key
         FROM ui_installations AS installation
         LEFT JOIN projects AS project ON project.id = installation.project_id
         WHERE installation.id = $1",
    )
    .bind(installation_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
}

pub(super) async fn lock_installation(
    tx: &mut Transaction<'_, Postgres>,
    installation_id: UiInstallationId,
) -> Result<Option<LockedInstallationRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT installation.project_id,
                COALESCE(installation.organization_id, project.organization_id)
                    AS organization_id,
                installation.repository_id, installation.scope, installation.ui_key,
                installation.lifecycle, installation.current_generation_id,
                current_generation.generation_no AS current_generation_no
         FROM ui_installations AS installation
         JOIN ui_installation_generations AS current_generation
           ON current_generation.id = installation.current_generation_id
          AND current_generation.installation_id = installation.id
         LEFT JOIN projects AS project ON project.id = installation.project_id
         WHERE installation.id = $1
         FOR UPDATE OF installation",
    )
    .bind(installation_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
}

pub(super) fn installation_target(
    owner: &InstallationOwnerRow,
) -> Option<release_domain::UiInstallationTarget> {
    match (owner.scope.as_str(), owner.project, owner.repository) {
        ("global", None, None) => Some(release_domain::UiInstallationTarget::organization(
            OrganizationId::from_uuid(owner.organization),
        )),
        ("project", Some(project_id), None) => Some(release_domain::UiInstallationTarget::project(
            ProjectId::from_uuid(project_id),
        )),
        ("repository", Some(_project_id), Some(repository_id)) => {
            Some(release_domain::UiInstallationTarget::repository(
                RepositoryId::from_uuid(repository_id),
            ))
        }
        _ => None,
    }
}

pub(super) fn same_owner(initial: &InstallationOwnerRow, locked: &LockedInstallationRow) -> bool {
    initial.project == locked.project
        && initial.organization == locked.organization
        && initial.repository == locked.repository
        && initial.scope == locked.scope
        && initial.ui_key == locked.ui_key
}

pub(super) async fn require_owner_management(
    service: &ReleaseService,
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    target: release_domain::UiInstallationTarget,
    owner: &OwnerRow,
) -> Result<(), UiInstallationError> {
    match target {
        release_domain::UiInstallationTarget::Organization(_) => service
            .require(
                tx,
                identity,
                Permission::CanManage,
                ObjectRef::new(ObjectType::Organization, owner.organization),
            )
            .await
            .map_err(|error| map_authorization_public(&error)),
        release_domain::UiInstallationTarget::Project(_)
        | release_domain::UiInstallationTarget::Repository(_) => {
            let project_id = owner.project.ok_or(UiInstallationError::Unavailable)?;
            service
                .require(
                    tx,
                    identity,
                    Permission::CanManage,
                    ObjectRef::new(ObjectType::Project, project_id),
                )
                .await
                .map_err(|error| map_authorization_public(&error))?;
            if let Some(repository_id) = owner.repository {
                service
                    .require(
                        tx,
                        identity,
                        Permission::CanWrite,
                        ObjectRef::new(ObjectType::Repository, repository_id),
                    )
                    .await
                    .map_err(|error| map_authorization_public(&error))?;
            }
            Ok(())
        }
    }
}

pub(super) fn lifecycle_state(value: &str) -> Option<UiInstallationState> {
    match value {
        "enabled" => Some(UiInstallationState::Enabled),
        "disabled" => Some(UiInstallationState::Disabled),
        "removed" => Some(UiInstallationState::Removed),
        _ => None,
    }
}

pub(super) const fn receipt_scope(target: UiInstallationTarget) -> UiInstallationReceiptScope {
    let aggregate_type = match target {
        UiInstallationTarget::Organization(_) => "organization",
        UiInstallationTarget::Project(_) => "project",
        UiInstallationTarget::Repository(_) => "repository",
    };
    UiInstallationReceiptScope {
        aggregate_type,
        primary_scope_kind: match target {
            UiInstallationTarget::Repository(_) => "project",
            UiInstallationTarget::Organization(_) | UiInstallationTarget::Project(_) => {
                aggregate_type
            }
        },
    }
}

pub(super) const fn lifecycle_name(value: UiInstallationState) -> &'static str {
    match value {
        UiInstallationState::Enabled => "enabled",
        UiInstallationState::Disabled => "disabled",
        UiInstallationState::Removed => "removed",
    }
}
