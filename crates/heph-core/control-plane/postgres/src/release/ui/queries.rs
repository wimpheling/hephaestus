use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::super::ReleaseError;
use super::types::{
    ApiBindingRow, DescriptorRow, ManagedServiceRow, ReleaseUiDescriptor, StaticFileRow,
};
use super::{
    MAX_RELEASE_UI_API_BINDINGS, MAX_RELEASE_UI_DESCRIPTORS, MAX_RELEASE_UI_STATIC_FILES,
    assembly::assemble,
};

/// Loads and validates immutable UI bindings inside the caller's actor transaction.
pub async fn load_release_ui_descriptors(
    transaction: &mut Transaction<'_, Postgres>,
    release_id: Uuid,
) -> Result<Vec<ReleaseUiDescriptor>, ReleaseError> {
    let descriptor_rows = sqlx::query_as::<_, DescriptorRow>(
        "SELECT ui_key, scope, label, icon, presentation, route_base, entrypoint,
                ui_kit_version, cache, repository_git_access, content_kind
         FROM release_ui_descriptors
         WHERE release_id = $1
           AND check_permission('user', hephaestus_actor_id(), 'can_read',
               'release', release_id::text) = 1
         ORDER BY ui_key
         LIMIT $2",
    )
    .bind(release_id)
    .bind(limit_plus_one(MAX_RELEASE_UI_DESCRIPTORS))
    .fetch_all(&mut **transaction)
    .await
    .map_err(ReleaseError::Persistence)?;
    if descriptor_rows.len() > MAX_RELEASE_UI_DESCRIPTORS {
        return Err(ReleaseError::InvalidStoredData);
    }

    let static_rows = sqlx::query_as::<_, StaticFileRow>(
        "SELECT ui_key, route, artifact_id, artifact_kind, artifact_media_type
         FROM release_ui_static_files
         WHERE release_id = $1
           AND check_permission('user', hephaestus_actor_id(), 'can_read',
               'release', release_id::text) = 1
         ORDER BY ui_key, route
         LIMIT $2",
    )
    .bind(release_id)
    .bind(limit_plus_one(MAX_RELEASE_UI_STATIC_FILES))
    .fetch_all(&mut **transaction)
    .await
    .map_err(ReleaseError::Persistence)?;
    if static_rows.len() > MAX_RELEASE_UI_STATIC_FILES {
        return Err(ReleaseError::InvalidStoredData);
    }

    let managed_rows = sqlx::query_as::<_, ManagedServiceRow>(
        "SELECT ui_key, gateway_name, route, release_agent_id
         FROM release_ui_managed_services
         WHERE release_id = $1
           AND check_permission('user', hephaestus_actor_id(), 'can_read',
               'release', release_id::text) = 1
         ORDER BY ui_key
         LIMIT $2",
    )
    .bind(release_id)
    .bind(limit_plus_one(MAX_RELEASE_UI_DESCRIPTORS))
    .fetch_all(&mut **transaction)
    .await
    .map_err(ReleaseError::Persistence)?;
    if managed_rows.len() > MAX_RELEASE_UI_DESCRIPTORS {
        return Err(ReleaseError::InvalidStoredData);
    }

    let api_rows = sqlx::query_as::<_, ApiBindingRow>(
        "SELECT ui_key, api_key, gateway_name, method, route, release_agent_id
         FROM release_ui_api_bindings
         WHERE release_id = $1
           AND check_permission('user', hephaestus_actor_id(), 'can_read',
               'release', release_id::text) = 1
         ORDER BY ui_key, api_key
         LIMIT $2",
    )
    .bind(release_id)
    .bind(limit_plus_one(MAX_RELEASE_UI_API_BINDINGS))
    .fetch_all(&mut **transaction)
    .await
    .map_err(ReleaseError::Persistence)?;
    if api_rows.len() > MAX_RELEASE_UI_API_BINDINGS {
        return Err(ReleaseError::InvalidStoredData);
    }

    assemble(descriptor_rows, static_rows, managed_rows, api_rows)
        .map_err(|()| ReleaseError::InvalidStoredData)
}

fn limit_plus_one(limit: usize) -> i64 {
    i64::try_from(limit + 1).expect("published UI limits fit in i64")
}
