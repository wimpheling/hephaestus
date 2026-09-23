//! Typed, authorized inspection of immutable release UI bindings.

use agent_config::ui::{MAX_UI_APIS, MAX_UI_FILES};
use release_domain::ui::{
    UiIcon, UiKey, UiLabel, UiMediaType, UiPresentation, UiRepositoryGitAccess, UiRoutePath,
    UiScope,
};
use sqlx::{FromRow, Postgres, Transaction};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Maximum published UI descriptors returned for one release.
pub const MAX_RELEASE_UI_DESCRIPTORS: usize = 16;
/// Maximum static file bindings returned for one release.
pub const MAX_RELEASE_UI_STATIC_FILES: usize = 4096;
/// Maximum API bindings returned for one release.
pub const MAX_RELEASE_UI_API_BINDINGS: usize = 256;

/// One immutable release UI descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseUiDescriptor {
    /// Stable UI key.
    pub key: UiKey,
    /// Installation scope.
    pub scope: UiScope,
    /// Host-shell label.
    pub label: UiLabel,
    /// Host-shell icon.
    pub icon: UiIcon,
    /// Initial host presentation.
    pub presentation: UiPresentation,
    /// Platform-relative route base.
    pub route_base: UiRoutePath,
    /// Relative content entrypoint.
    pub entrypoint: UiRoutePath,
    /// Version of the published UI kit contract.
    pub ui_kit_version: u16,
    /// Browser/intermediary cache policy.
    pub cache: UiCachePolicy,
    /// Explicit generic repository Git authority.
    pub repository_git_access: UiRepositoryGitAccess,
    /// Immutable content binding.
    pub content: ReleaseUiContent,
    /// Explicit gateway API bindings.
    pub apis: Vec<ReleaseUiApiBinding>,
}

/// Cache policy stored for a published release UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiCachePolicy {
    /// Do not permit browser or intermediary caching.
    NoStore,
}

/// Immutable content binding for a release UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseUiContent {
    /// Static files bound to exact release artifact IDs.
    Static {
        /// Route-to-artifact bindings.
        files: Vec<ReleaseUiStaticFile>,
    },
    /// Managed service bound to one exact release agent ID.
    ManagedService {
        /// Repository gateway name.
        gateway_name: String,
        /// Absolute gateway route.
        route: String,
        /// Exact release agent identity.
        release_agent_id: Uuid,
    },
}

/// One immutable static route binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseUiStaticFile {
    /// UI-relative route.
    pub route: UiRoutePath,
    /// Exact release artifact identity.
    pub artifact_id: Uuid,
    /// Explicit artifact MIME type.
    pub media_type: UiMediaType,
}

/// One immutable gateway API binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseUiApiBinding {
    /// UI-local API key.
    pub key: UiKey,
    /// Repository gateway name.
    pub gateway_name: String,
    /// Exact HTTP method.
    pub method: String,
    /// Absolute gateway route.
    pub route: String,
    /// Exact release agent identity.
    pub release_agent_id: Uuid,
}

#[derive(Debug, FromRow)]
struct DescriptorRow {
    ui_key: String,
    scope: String,
    label: String,
    icon: String,
    presentation: String,
    route_base: String,
    entrypoint: String,
    ui_kit_version: i32,
    cache: String,
    repository_git_access: String,
    content_kind: String,
}

#[derive(Debug, FromRow)]
struct StaticFileRow {
    ui_key: String,
    route: String,
    artifact_id: Uuid,
    artifact_kind: String,
    artifact_media_type: String,
}

#[derive(Debug, FromRow)]
struct ManagedServiceRow {
    ui_key: String,
    gateway_name: String,
    route: String,
    release_agent_id: Uuid,
}

#[derive(Debug, FromRow)]
struct ApiBindingRow {
    ui_key: String,
    api_key: String,
    gateway_name: String,
    method: String,
    route: String,
    release_agent_id: Uuid,
}

/// Loads and validates immutable UI bindings inside the caller's actor transaction.
pub(crate) async fn load_release_ui_descriptors(
    transaction: &mut Transaction<'_, Postgres>,
    release_id: Uuid,
) -> Result<Vec<ReleaseUiDescriptor>, super::ReleaseError> {
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
    .map_err(super::ReleaseError::Persistence)?;
    if descriptor_rows.len() > MAX_RELEASE_UI_DESCRIPTORS {
        return Err(super::ReleaseError::InvalidStoredData);
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
    .map_err(super::ReleaseError::Persistence)?;
    if static_rows.len() > MAX_RELEASE_UI_STATIC_FILES {
        return Err(super::ReleaseError::InvalidStoredData);
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
    .map_err(super::ReleaseError::Persistence)?;
    if managed_rows.len() > MAX_RELEASE_UI_DESCRIPTORS {
        return Err(super::ReleaseError::InvalidStoredData);
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
    .map_err(super::ReleaseError::Persistence)?;
    if api_rows.len() > MAX_RELEASE_UI_API_BINDINGS {
        return Err(super::ReleaseError::InvalidStoredData);
    }

    assemble(descriptor_rows, static_rows, managed_rows, api_rows)
        .map_err(|()| super::ReleaseError::InvalidStoredData)
}

fn limit_plus_one(limit: usize) -> i64 {
    i64::try_from(limit + 1).expect("published UI limits fit in i64")
}

fn assemble(
    descriptors: Vec<DescriptorRow>,
    static_rows: Vec<StaticFileRow>,
    managed_rows: Vec<ManagedServiceRow>,
    api_rows: Vec<ApiBindingRow>,
) -> Result<Vec<ReleaseUiDescriptor>, ()> {
    let descriptors_by_key = parse_descriptors(descriptors)?;
    let mut static_by_key = parse_static_files(static_rows)?;
    let mut managed_by_key = parse_managed_services(managed_rows)?;
    let mut api_by_key = parse_api_bindings(api_rows)?;

    let mut result = BTreeMap::new();
    for (key, metadata) in descriptors_by_key {
        let content = match metadata.content_kind.as_str() {
            "static" => {
                let files = static_by_key.remove(&key).ok_or(())?;
                if files.len() > MAX_UI_FILES
                    || files
                        .iter()
                        .filter(|file| file.route == metadata.entrypoint)
                        .count()
                        != 1
                    || !files.iter().any(|file| {
                        file.route == metadata.entrypoint
                            && file.media_type == UiMediaType::TextHtml
                    })
                {
                    return Err(());
                }
                if managed_by_key.contains_key(&key) {
                    return Err(());
                }
                ReleaseUiContent::Static { files }
            }
            "managed_service" => {
                if static_by_key.contains_key(&key) {
                    return Err(());
                }
                let mut managed = managed_by_key.remove(&key).ok_or(())?;
                if managed.len() != 1 {
                    return Err(());
                }
                managed.pop().ok_or(())?
            }
            _ => return Err(()),
        };
        let apis = api_by_key.remove(&key).unwrap_or_default();
        if apis.len() > MAX_UI_APIS {
            return Err(());
        }
        result.insert(
            key.clone(),
            ReleaseUiDescriptor {
                key,
                scope: metadata.scope,
                label: metadata.label,
                icon: metadata.icon,
                presentation: metadata.presentation,
                route_base: metadata.route_base,
                entrypoint: metadata.entrypoint,
                ui_kit_version: metadata.ui_kit_version,
                cache: metadata.cache,
                repository_git_access: metadata.repository_git_access,
                content,
                apis,
            },
        );
    }
    if !static_by_key.is_empty() || !managed_by_key.is_empty() || !api_by_key.is_empty() {
        return Err(());
    }
    Ok(result.into_values().collect())
}

fn parse_descriptors(rows: Vec<DescriptorRow>) -> Result<BTreeMap<UiKey, DescriptorMetadata>, ()> {
    rows.into_iter()
        .map(|row| {
            let key = UiKey::parse(row.ui_key).map_err(|_| ())?;
            if !matches!(row.content_kind.as_str(), "static" | "managed_service") {
                return Err(());
            }
            let metadata = DescriptorMetadata {
                scope: parse_scope(&row.scope)?,
                label: UiLabel::parse(row.label).map_err(|_| ())?,
                icon: parse_icon(&row.icon)?,
                presentation: parse_presentation(&row.presentation)?,
                route_base: UiRoutePath::parse(row.route_base).map_err(|_| ())?,
                entrypoint: UiRoutePath::parse(row.entrypoint).map_err(|_| ())?,
                ui_kit_version: u16::try_from(row.ui_kit_version).map_err(|_| ())?,
                cache: parse_cache(&row.cache)?,
                repository_git_access: UiRepositoryGitAccess::parse(row.repository_git_access)
                    .map_err(|_| ())?,
                content_kind: row.content_kind,
            };
            if metadata.ui_kit_version != 1 {
                return Err(());
            }
            Ok((key, metadata))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()
}

fn parse_static_files(
    rows: Vec<StaticFileRow>,
) -> Result<BTreeMap<UiKey, Vec<ReleaseUiStaticFile>>, ()> {
    let mut grouped = BTreeMap::new();
    for row in rows {
        let key = UiKey::parse(row.ui_key).map_err(|_| ())?;
        if row.artifact_kind != "file" {
            return Err(());
        }
        grouped
            .entry(key)
            .or_insert_with(Vec::new)
            .push(ReleaseUiStaticFile {
                route: UiRoutePath::parse(row.route).map_err(|_| ())?,
                artifact_id: row.artifact_id,
                media_type: UiMediaType::try_from(row.artifact_media_type).map_err(|_| ())?,
            });
    }
    Ok(grouped)
}

fn parse_managed_services(
    rows: Vec<ManagedServiceRow>,
) -> Result<BTreeMap<UiKey, Vec<ReleaseUiContent>>, ()> {
    let mut grouped = BTreeMap::new();
    for row in rows {
        let key = UiKey::parse(row.ui_key).map_err(|_| ())?;
        if !valid_gateway_name(&row.gateway_name) {
            return Err(());
        }
        grouped
            .entry(key)
            .or_insert_with(Vec::new)
            .push(ReleaseUiContent::ManagedService {
                gateway_name: row.gateway_name,
                route: parse_absolute_route(row.route)?,
                release_agent_id: row.release_agent_id,
            });
    }
    Ok(grouped)
}

fn parse_api_bindings(
    rows: Vec<ApiBindingRow>,
) -> Result<BTreeMap<UiKey, Vec<ReleaseUiApiBinding>>, ()> {
    let mut grouped = BTreeMap::new();
    for row in rows {
        let key = UiKey::parse(row.ui_key).map_err(|_| ())?;
        if !valid_gateway_name(&row.gateway_name) || !valid_http_method(&row.method) {
            return Err(());
        }
        grouped
            .entry(key)
            .or_insert_with(Vec::new)
            .push(ReleaseUiApiBinding {
                key: UiKey::parse(row.api_key).map_err(|_| ())?,
                gateway_name: row.gateway_name,
                method: row.method,
                route: parse_absolute_route(row.route)?,
                release_agent_id: row.release_agent_id,
            });
    }
    Ok(grouped)
}

struct DescriptorMetadata {
    scope: UiScope,
    label: UiLabel,
    icon: UiIcon,
    presentation: UiPresentation,
    route_base: UiRoutePath,
    entrypoint: UiRoutePath,
    ui_kit_version: u16,
    cache: UiCachePolicy,
    repository_git_access: UiRepositoryGitAccess,
    content_kind: String,
}

fn parse_scope(value: &str) -> Result<UiScope, ()> {
    match value {
        "project" => Ok(UiScope::Project),
        "repository" => Ok(UiScope::Repository),
        "global" => Ok(UiScope::Global),
        _ => Err(()),
    }
}

fn parse_icon(value: &str) -> Result<UiIcon, ()> {
    match value {
        "app" => Ok(UiIcon::App),
        "chat" => Ok(UiIcon::Chat),
        "code" => Ok(UiIcon::Code),
        "book" => Ok(UiIcon::Book),
        "chart" => Ok(UiIcon::Chart),
        _ => Err(()),
    }
}

fn parse_presentation(value: &str) -> Result<UiPresentation, ()> {
    match value {
        "iframe" => Ok(UiPresentation::Iframe),
        "full_page" => Ok(UiPresentation::FullPage),
        _ => Err(()),
    }
}

fn parse_cache(value: &str) -> Result<UiCachePolicy, ()> {
    match value {
        "no_store" => Ok(UiCachePolicy::NoStore),
        _ => Err(()),
    }
}

fn valid_http_method(value: &str) -> bool {
    matches!(
        value,
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
    )
}

fn parse_absolute_route(value: String) -> Result<String, ()> {
    let relative = value.strip_prefix('/').ok_or(())?;
    UiRoutePath::parse(relative.to_owned()).map_err(|_| ())?;
    Ok(value)
}

fn valid_gateway_name(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value.bytes().enumerate().all(|(index, byte)| {
            (index == 0 && byte.is_ascii_lowercase())
                || (index > 0 && (byte.is_ascii_lowercase() || byte.is_ascii_digit()))
                || (index > 0 && matches!(byte, b'_' | b'-'))
        })
}
