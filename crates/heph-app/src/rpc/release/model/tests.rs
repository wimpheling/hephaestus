use super::ui::{ui_descriptor, ui_descriptors};
use crate::application::release::ui::{
    ReleaseUiApiBinding as ApplicationUiApiBinding, ReleaseUiContent as ApplicationUiContent,
    ReleaseUiDescriptor as ApplicationUiDescriptor, ReleaseUiStaticFile as ApplicationUiStaticFile,
    UiCachePolicy as ApplicationUiCachePolicy,
};
use release_domain::ui::{
    UiIcon, UiKey, UiLabel, UiMediaType, UiPresentation, UiRepositoryGitAccess, UiRoutePath,
    UiScope,
};
use rpc_proto::messages::hephaestus::release::v1::release_ui_descriptor::Content;
use uuid::Uuid;

fn metadata(key: &str) -> ApplicationUiDescriptor {
    ApplicationUiDescriptor {
        key: UiKey::parse(key).expect("valid UI key"),
        scope: UiScope::Repository,
        label: UiLabel::parse("Repository UI").expect("valid label"),
        icon: UiIcon::Code,
        presentation: UiPresentation::Iframe,
        route_base: UiRoutePath::parse("tools").expect("valid route base"),
        entrypoint: UiRoutePath::parse("index.html").expect("valid entrypoint"),
        ui_kit_version: 1,
        cache: ApplicationUiCachePolicy::NoStore,
        repository_git_access: UiRepositoryGitAccess::None,
        content: ApplicationUiContent::Static { files: Vec::new() },
        apis: Vec::new(),
    }
}

#[test]
fn ui_mapper_preserves_static_artifact_and_api_identity() {
    let artifact_id = Uuid::from_u128(1);
    let agent_id = Uuid::from_u128(2);
    let mut value = metadata("tools");
    value.content = ApplicationUiContent::Static {
        files: vec![ApplicationUiStaticFile {
            route: UiRoutePath::parse("index.html").expect("valid file route"),
            artifact_id,
            media_type: UiMediaType::TextHtml,
        }],
    };
    value.apis.push(ApplicationUiApiBinding {
        key: UiKey::parse("health").expect("valid API key"),
        gateway_name: "tools-api".to_owned(),
        method: "GET".to_owned(),
        route: "/health".to_owned(),
        release_agent_id: agent_id,
    });

    let mapped = ui_descriptor(value);
    assert_eq!(mapped.key, "tools");
    assert_eq!(mapped.route_base, "tools");
    assert_eq!(
        mapped.apis[0].release_agent_id.as_option().unwrap().value,
        agent_id.to_string()
    );
    match mapped.content.expect("static content") {
        Content::StaticContent(content) => {
            assert_eq!(
                content.files[0].artifact_id.as_option().unwrap().value,
                artifact_id.to_string()
            );
            assert_eq!(content.files[0].route, "index.html");
            assert_eq!(content.files[0].media_type, "text/html");
        }
        Content::ManagedService(_) => panic!("expected static content"),
    }
}

#[test]
fn ui_mapper_preserves_managed_service_identity_and_legacy_empty() {
    let agent_id = Uuid::from_u128(3);
    let mut value = metadata("service");
    value.content = ApplicationUiContent::ManagedService {
        gateway_name: "service".to_owned(),
        route: "/service".to_owned(),
        release_agent_id: agent_id,
    };

    let mapped = ui_descriptor(value);
    match mapped.content.expect("managed content") {
        Content::ManagedService(service) => {
            assert_eq!(service.gateway_name, "service");
            assert_eq!(service.route, "/service");
            assert_eq!(
                service.release_agent_id.as_option().unwrap().value,
                agent_id.to_string()
            );
        }
        Content::StaticContent(_) => panic!("expected managed content"),
    }
    assert!(ui_descriptors(Vec::new()).is_empty());
}
