use super::opaque;
use crate::application::release::ui::{
    ReleaseUiApiBinding as ApplicationUiApiBinding, ReleaseUiContent as ApplicationUiContent,
    ReleaseUiDescriptor as ApplicationUiDescriptor, ReleaseUiStaticFile as ApplicationUiStaticFile,
    UiCachePolicy as ApplicationUiCachePolicy,
};
use rpc_proto::messages::hephaestus::release::v1::{
    ReleaseUiApiBinding, ReleaseUiCachePolicy, ReleaseUiDescriptor, ReleaseUiIcon,
    ReleaseUiManagedService, ReleaseUiPresentation, ReleaseUiRepositoryGitAccess, ReleaseUiScope,
    ReleaseUiStaticContent, ReleaseUiStaticFile, release_ui_descriptor,
};

pub fn ui_descriptors(values: Vec<ApplicationUiDescriptor>) -> Vec<ReleaseUiDescriptor> {
    values.into_iter().map(ui_descriptor).collect()
}

pub(super) fn ui_descriptor(value: ApplicationUiDescriptor) -> ReleaseUiDescriptor {
    let content = match value.content {
        ApplicationUiContent::Static { files } => {
            release_ui_descriptor::Content::StaticContent(Box::new(ReleaseUiStaticContent {
                files: files.into_iter().map(|file| static_file(&file)).collect(),
                ..Default::default()
            }))
        }
        ApplicationUiContent::ManagedService {
            gateway_name,
            route,
            release_agent_id,
        } => release_ui_descriptor::Content::ManagedService(Box::new(ReleaseUiManagedService {
            gateway_name,
            route,
            release_agent_id: opaque(release_agent_id).into(),
            ..Default::default()
        })),
    };
    ReleaseUiDescriptor {
        key: value.key.to_string(),
        scope: ui_scope(value.scope).into(),
        label: value.label.to_string(),
        icon: ui_icon(value.icon).into(),
        presentation: ui_presentation(value.presentation).into(),
        route_base: value.route_base.to_string(),
        entrypoint: value.entrypoint.to_string(),
        ui_kit_version: u32::from(value.ui_kit_version),
        cache: ui_cache(value.cache).into(),
        repository_git_access: ui_repository_git_access(value.repository_git_access).into(),
        content: Some(content),
        apis: value.apis.into_iter().map(api_binding).collect(),
        ..Default::default()
    }
}

const fn ui_repository_git_access(
    value: release_domain::ui::UiRepositoryGitAccess,
) -> ReleaseUiRepositoryGitAccess {
    match value {
        release_domain::ui::UiRepositoryGitAccess::None => ReleaseUiRepositoryGitAccess::None,
        release_domain::ui::UiRepositoryGitAccess::Read => ReleaseUiRepositoryGitAccess::Read,
        release_domain::ui::UiRepositoryGitAccess::ReadWrite => {
            ReleaseUiRepositoryGitAccess::ReadWrite
        }
    }
}

fn static_file(value: &ApplicationUiStaticFile) -> ReleaseUiStaticFile {
    ReleaseUiStaticFile {
        route: value.route.to_string(),
        artifact_id: opaque(value.artifact_id).into(),
        media_type: value.media_type.into(),
        ..Default::default()
    }
}

fn api_binding(value: ApplicationUiApiBinding) -> ReleaseUiApiBinding {
    ReleaseUiApiBinding {
        key: value.key.to_string(),
        gateway_name: value.gateway_name,
        method: value.method,
        route: value.route,
        release_agent_id: opaque(value.release_agent_id).into(),
        ..Default::default()
    }
}

const fn ui_scope(value: release_domain::ui::UiScope) -> ReleaseUiScope {
    match value {
        release_domain::ui::UiScope::Project => ReleaseUiScope::Project,
        release_domain::ui::UiScope::Repository => ReleaseUiScope::Repository,
        release_domain::ui::UiScope::Global => ReleaseUiScope::Global,
    }
}

const fn ui_icon(value: release_domain::ui::UiIcon) -> ReleaseUiIcon {
    match value {
        release_domain::ui::UiIcon::App => ReleaseUiIcon::App,
        release_domain::ui::UiIcon::Chat => ReleaseUiIcon::Chat,
        release_domain::ui::UiIcon::Code => ReleaseUiIcon::Code,
        release_domain::ui::UiIcon::Book => ReleaseUiIcon::Book,
        release_domain::ui::UiIcon::Chart => ReleaseUiIcon::Chart,
    }
}

const fn ui_presentation(value: release_domain::ui::UiPresentation) -> ReleaseUiPresentation {
    match value {
        release_domain::ui::UiPresentation::Iframe => ReleaseUiPresentation::Iframe,
        release_domain::ui::UiPresentation::FullPage => ReleaseUiPresentation::FullPage,
    }
}

const fn ui_cache(value: ApplicationUiCachePolicy) -> ReleaseUiCachePolicy {
    match value {
        ApplicationUiCachePolicy::NoStore => ReleaseUiCachePolicy::NoStore,
    }
}
