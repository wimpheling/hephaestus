use std::collections::BTreeMap;

use forge_domain::ProjectId;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use recipe_domain::{ReleaseCatalogEntry, ReleasePin, parse_recipe};
use release_domain::{
    ContentHash, NetworkAccess, ParameterDeclaration, ParameterName, ParameterType, ParameterValue,
    ReleaseAgentId, ReleaseId, RuntimePolicy,
};
use serde_json::json;
use uuid::Uuid;
use volume_domain::{GuestMountPath, VolumeAccessMode, VolumeSlotDeclaration};

use crate::{
    CommandIdentity, DeploymentExecutionPreparation, DeploymentExecutionProfile, DeploymentId,
    DeploymentIntent, DeploymentKey, DeploymentOperation, PlatformPolicyObservation,
    SourceObservation,
};

pub const SOURCE: &str = r#"
contract_version = 1
recipe_id = "sqlite"
recipe_version = "1.0.0"
[[resources]]
kind = "volume"
name = "data"
source = { type = "created", capacity_bytes = { source = "literal", value = 16777216 } }
[[resources]]
kind = "volume"
name = "read_data"
source = { type = "created", capacity_bytes = { source = "literal", value = 16777216 } }
[[resources]]
kind = "instance"
name = "writer"
release = { release_id = "10000000-0000-4000-8000-000000000001", release_agent_id = "10000000-0000-4000-8000-000000000002" }
volume_bindings = [{ slot = "state", resource = "data", guest_path = "/data", access_mode = "read_write" }]
[[resources]]
kind = "instance"
name = "reader"
release = { release_id = "10000000-0000-4000-8000-000000000001", release_agent_id = "10000000-0000-4000-8000-000000000002" }
volume_bindings = [{ slot = "state", resource = "read_data", guest_path = "/data", access_mode = "read_write" }]
"#;

pub fn identity(actor: u128, request: u128) -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        UserId::from_uuid(Uuid::from_u128(actor)),
        "test",
        "creator",
        json!({}),
        RequestId::from_uuid(Uuid::from_u128(request)),
    )
    .with_idempotency_id(RequestId::from_uuid(Uuid::from_u128(99)))
}

pub fn command() -> CommandIdentity {
    CommandIdentity::from_identity(&identity(7, 8), DeploymentOperation::Install).unwrap()
}

pub fn policy(memory_mib: u32) -> RuntimePolicy {
    RuntimePolicy {
        vcpus: 1,
        memory_mib,
        network: NetworkAccess::Disabled,
    }
}

pub fn platform(version: &str) -> PlatformPolicyObservation {
    PlatformPolicyObservation::new(policy(512), version.to_owned()).unwrap()
}

pub fn pin() -> ReleasePin {
    ReleasePin {
        release_id: ReleaseId::from_uuid(
            Uuid::parse_str("10000000-0000-4000-8000-000000000001").unwrap(),
        ),
        release_agent_id: ReleaseAgentId::from_uuid(
            Uuid::parse_str("10000000-0000-4000-8000-000000000002").unwrap(),
        ),
    }
}

pub fn observation() -> SourceObservation {
    SourceObservation::new(
        pin(),
        ContentHash::digest(b"authored contract"),
        &format!("registry.example/sqlite@sha256:{}", "a".repeat(64)),
        policy(128),
        platform("platform/v1"),
    )
    .unwrap()
}

pub fn intent(source: &str, deployment: u128) -> DeploymentIntent {
    with_catalog(source, deployment, catalog(), &BTreeMap::new())
}

pub fn catalog() -> ReleaseCatalogEntry {
    ReleaseCatalogEntry {
        pin: pin(),
        published: true,
        parameters: vec![ParameterDeclaration {
            name: ParameterName::parse("message").unwrap(),
            value_type: ParameterType::String {
                minimum_length: 0,
                maximum_length: 4096,
            },
            required: true,
            default: Some(ParameterValue::String("hello".to_owned())),
            sensitive: false,
        }],
        volume_slots: vec![
            VolumeSlotDeclaration::new(
                capability_domain::CapabilitySlotKey::parse("state").unwrap(),
                GuestMountPath::parse("/data").unwrap(),
                VolumeAccessMode::ReadWrite,
                true,
                16_777_216,
            )
            .unwrap(),
        ],
        required_secret_slot_count: 0,
        capability_requirements: vec![],
    }
}

pub fn with_catalog(
    source: &str,
    deployment: u128,
    catalog: ReleaseCatalogEntry,
    external: &BTreeMap<capability_domain::CapabilitySlotKey, recipe_domain::ExternalVolume>,
) -> DeploymentIntent {
    let declaration = parse_recipe(source.as_bytes()).unwrap();
    let resolved = declaration
        .resolve(&BTreeMap::new(), external, &[catalog])
        .unwrap();
    DeploymentIntent::new(
        DeploymentId::from_uuid(Uuid::from_u128(deployment)).unwrap(),
        ProjectId::from_uuid(Uuid::from_u128(2)),
        DeploymentKey::parse("sqlite").unwrap(),
        &declaration,
        &resolved,
    )
    .unwrap()
}

pub fn prepared() -> DeploymentExecutionPreparation {
    DeploymentExecutionPreparation::new(
        &intent(SOURCE, 1),
        command(),
        DeploymentExecutionProfile::RuntimeNamedV1,
        platform("platform/v1"),
        &[observation()],
    )
    .unwrap()
}
