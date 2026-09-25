use crate::application::release::ui::{
    ReleaseUiApiBinding as ApplicationUiApiBinding, ReleaseUiContent as ApplicationUiContent,
    ReleaseUiDescriptor as ApplicationUiDescriptor, ReleaseUiStaticFile as ApplicationUiStaticFile,
    UiCachePolicy as ApplicationUiCachePolicy,
};
use crate::application::{
    build::BuildView,
    release::{
        ReleaseAgent as ApplicationAgent, ReleaseArtifact as ApplicationArtifact, ReleaseError,
        ReleaseState as ApplicationState, ReleaseSummary as ApplicationSummary,
    },
};
use agent_config::{SecretDeliveryMode, SecretPhase, SecretSlotDeclaration};
use release_domain::{
    NetworkAccess, ParameterDeclaration as ApplicationParameter, ParameterType, ParameterValue,
};
use rpc_proto::messages::hephaestus::{
    artifact::v1::{Artifact, ArtifactProvenance},
    common::v1::{
        BooleanParameterConstraints, EnumParameterConstraints, IntegerParameterConstraints,
        NetworkPolicy, OpaqueId, ParameterDeclaration, ParameterDefault,
        ParameterType as ProtoParameterType, RuntimeContract, RuntimePolicy,
        SecretSlotDeclaration as ProtoSecretSlot, SecretSlotDeliveryMode, SecretSlotPhase,
        StringParameterConstraints, UpdateHook, parameter_default,
    },
    release::v1::{
        Release, ReleaseAgent, ReleaseState, ReleaseSummary, ReleaseUiApiBinding,
        ReleaseUiCachePolicy, ReleaseUiDescriptor, ReleaseUiIcon, ReleaseUiManagedService,
        ReleaseUiPresentation, ReleaseUiRepositoryGitAccess, ReleaseUiScope,
        ReleaseUiStaticContent, ReleaseUiStaticFile, release_ui_descriptor,
    },
};
use time::OffsetDateTime;
use uuid::Uuid;

pub(super) fn summary(value: ApplicationSummary) -> ReleaseSummary {
    ReleaseSummary {
        id: opaque(value.id).into(),
        version: value.version,
        state: state(value.state).into(),
        source_commit: value.source_commit,
        source_ref: value.source_ref,
        build_request_id: opaque(value.build_request_id).into(),
        created_at: timestamp(value.created_at).into(),
        published_at: value.published_at.map(timestamp).into(),
        manifest_hash: value.manifest_hash,
        artifact_count: value.artifact_count,
        exported_agent_count: value.agent_count,
        ..Default::default()
    }
}

pub(super) fn release(value: crate::application::release::ReleaseDetail) -> Release {
    let release_id = value.summary.id;
    let build_id = value.summary.build_request_id;
    let source_commit = value.summary.source_commit.clone();
    Release {
        id: opaque(release_id).into(),
        version: value.summary.version,
        state: state(value.summary.state).into(),
        source_commit: value.summary.source_commit,
        source_ref: value.summary.source_ref,
        build_request_id: opaque(build_id).into(),
        build_definition_hash: value.build_definition_hash,
        configuration_hash: value.configuration_hash,
        manifest_hash: value.summary.manifest_hash,
        created_at: timestamp(value.summary.created_at).into(),
        published_at: value.summary.published_at.map(timestamp).into(),
        revoked_at: value.revoked_at.map(timestamp).into(),
        repository_id: opaque(value.repository_id).into(),
        repository_name: value.repository_name,
        project_id: opaque(value.project_id).into(),
        project_name: value.project_name,
        organization_id: opaque(value.organization_id).into(),
        organization_name: value.organization_name,
        build: build(value.build).into(),
        artifacts: value
            .artifacts
            .into_iter()
            .map(|artifact_value| artifact(artifact_value, release_id, build_id, &source_commit))
            .collect(),
        agents: value.agents.into_iter().map(agent).collect(),
        ui_descriptors: ui_descriptors(value.ui_descriptors),
        ..Default::default()
    }
}

fn ui_descriptors(values: Vec<ApplicationUiDescriptor>) -> Vec<ReleaseUiDescriptor> {
    values.into_iter().map(ui_descriptor).collect()
}

fn ui_descriptor(value: ApplicationUiDescriptor) -> ReleaseUiDescriptor {
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

pub(super) fn artifact(
    value: ApplicationArtifact,
    release_id: Uuid,
    build_id: Uuid,
    source_commit: &str,
) -> Artifact {
    Artifact {
        id: opaque(value.id).into(),
        path: value.path,
        kind: value.kind,
        mode: value.mode,
        sha256: value.sha256,
        size_bytes: value.size_bytes,
        media_type: value.media_type,
        provenance: ArtifactProvenance {
            build_id: opaque(build_id).into(),
            release_id: opaque(release_id).into(),
            source_commit: source_commit.to_owned(),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    }
}

pub(super) fn agent(value: ApplicationAgent) -> ReleaseAgent {
    let update_hook = value.update_hook.as_ref().map(|hook| UpdateHook {
        required: true,
        timeout_seconds: hook.timeout_seconds,
        ..Default::default()
    });
    ReleaseAgent {
        id: opaque(value.id).into(),
        family_id: opaque(value.family_id).into(),
        agent_key: value.agent_key,
        display_name: value.display_name,
        runtime_contract: RuntimeContract {
            policy_ceiling: runtime_policy(&value.policy).into(),
            requires_state: value.requires_state,
            ..Default::default()
        }
        .into(),
        parameter_schema: value.parameter_schema.into_iter().map(parameter).collect(),
        secret_slot_schema: value.secret_slots.into_iter().map(secret_slot).collect(),
        requires_state: value.requires_state,
        update_hook: update_hook.into(),
        created_at: timestamp(value.created_at).into(),
        ..Default::default()
    }
}

fn runtime_policy(value: &release_domain::RuntimePolicy) -> RuntimePolicy {
    RuntimePolicy {
        vcpus: u32::from(value.vcpus),
        memory_mib: value.memory_mib,
        network: match value.network {
            NetworkAccess::Disabled => NetworkPolicy::Disabled,
            NetworkAccess::BrokerOnly => NetworkPolicy::BrokerOnly,
            NetworkAccess::Egress => NetworkPolicy::Egress,
        }
        .into(),
        ..Default::default()
    }
}

fn parameter(value: ApplicationParameter) -> ParameterDeclaration {
    let name = value.name.to_string();
    let constraint = match value.value_type {
        ParameterType::String {
            minimum_length,
            maximum_length,
        } => StringParameterConstraints {
            minimum_length: u32::from(minimum_length),
            maximum_length: u32::from(maximum_length),
            ..Default::default()
        }
        .into(),
        ParameterType::Integer { minimum, maximum } => IntegerParameterConstraints {
            minimum,
            maximum,
            ..Default::default()
        }
        .into(),
        ParameterType::Boolean => BooleanParameterConstraints::default().into(),
        ParameterType::Enum { values } => EnumParameterConstraints {
            values,
            ..Default::default()
        }
        .into(),
    };
    let default = value.default.map(|value| ParameterDefault {
        value: Some(match value {
            ParameterValue::String(value) => parameter_default::Value::StringValue(value),
            ParameterValue::Integer(value) => parameter_default::Value::IntegerValue(value),
            ParameterValue::Boolean(value) => parameter_default::Value::BooleanValue(value),
        }),
        ..Default::default()
    });
    ParameterDeclaration {
        label: name.clone(),
        name,
        value_type: ProtoParameterType {
            constraint: Some(constraint),
            ..Default::default()
        }
        .into(),
        required: value.required,
        default: default.into(),
        sensitive: value.sensitive,
        ..Default::default()
    }
}

fn secret_slot(value: SecretSlotDeclaration) -> ProtoSecretSlot {
    ProtoSecretSlot {
        key: value.key,
        purpose: value.purpose,
        required: value.required,
        delivery_modes: value
            .delivery_modes
            .into_iter()
            .map(|mode| {
                match mode {
                    SecretDeliveryMode::Raw => SecretSlotDeliveryMode::Raw,
                    SecretDeliveryMode::Brokered => SecretSlotDeliveryMode::Brokered,
                }
                .into()
            })
            .collect(),
        phases: value
            .phases
            .into_iter()
            .map(|phase| {
                match phase {
                    SecretPhase::Normal => SecretSlotPhase::Normal,
                    SecretPhase::Update => SecretSlotPhase::Update,
                }
                .into()
            })
            .collect(),
        destinations: value.destinations,
        ..Default::default()
    }
}

pub(super) fn build(value: BuildView) -> rpc_proto::messages::hephaestus::build::v1::Build {
    super::super::build::model::build(value)
}

pub(super) const fn state(value: ApplicationState) -> ReleaseState {
    match value {
        ApplicationState::Draft => ReleaseState::Draft,
        ApplicationState::Published => ReleaseState::Published,
        ApplicationState::Revoked => ReleaseState::Revoked,
    }
}

pub(super) fn opaque(id: Uuid) -> OpaqueId {
    OpaqueId {
        value: id.to_string(),
        ..Default::default()
    }
}

pub(super) fn timestamp(value: OffsetDateTime) -> buffa_types::google::protobuf::Timestamp {
    buffa_types::google::protobuf::Timestamp {
        seconds: value.unix_timestamp(),
        nanos: i32::try_from(value.nanosecond()).unwrap_or_default(),
        ..Default::default()
    }
}

pub(super) fn application_error(error: ReleaseError) -> super::super::RpcError {
    use super::super::RpcError;

    match error {
        ReleaseError::NotFound => RpcError::NotFound,
        ReleaseError::InvalidPage | ReleaseError::InvalidVersion => RpcError::InvalidArgument,
        ReleaseError::Conflict => RpcError::AlreadyExists,
        ReleaseError::FailedPrecondition => RpcError::FailedPrecondition,
        ReleaseError::InvalidStoredData | ReleaseError::Serialization(_) => {
            tracing::error!(%error, "stored release data could not be represented");
            RpcError::Internal
        }
        ReleaseError::Persistence(source) => {
            tracing::error!(error = %source, "release application persistence failed");
            RpcError::Unavailable
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
