use super::rows::{
    ApiRow, BindingRow, GatewayRow, InstallationDiscovery, InstallationRow, ManagedRow, SourceRow,
};
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::PostgresMelangeAuthorizer;
use identity_domain::UserId;
use release_service::UiBrowserHandoffError;
use sqlx::{Postgres, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub(super) async fn require_permission(
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

pub(super) async fn lock_owner(
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
pub(super) async fn check_bindings(
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
