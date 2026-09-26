use agent_config::ui::{MAX_UI_APIS, MAX_UI_FILES};
use release_domain::ui::{
    UiIcon, UiKey, UiLabel, UiMediaType, UiPresentation, UiRepositoryGitAccess, UiRoutePath,
    UiScope,
};
use std::collections::BTreeMap;

use super::types::{
    ApiBindingRow, DescriptorRow, ManagedServiceRow, ReleaseUiApiBinding, ReleaseUiContent,
    ReleaseUiDescriptor, ReleaseUiStaticFile, StaticFileRow, UiCachePolicy,
};

pub(super) fn assemble(
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
