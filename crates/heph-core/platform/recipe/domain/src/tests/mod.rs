mod declarations;
mod resolution;

use capability_domain::{
    CapabilityOperation, CapabilityRequirement, CapabilityRequirementId, CapabilityResourceKind,
    CapabilitySlotKey,
};
use release_domain::{ParameterDeclaration, ParameterName, ParameterType};
use volume_domain::{GuestMountPath, VolumeAccessMode, VolumeSlotDeclaration};

use crate::{RecipeError, ReleaseCatalogEntry, ResourceDeclaration, parse_recipe};

const SAMPLE: &str = include_str!("../../examples/local-sqlite.toml");

fn key(value: &str) -> CapabilitySlotKey {
    CapabilitySlotKey::parse(value).expect("valid test key")
}

fn catalog() -> ReleaseCatalogEntry {
    let recipe = parse_recipe(SAMPLE.as_bytes()).expect("valid example");
    let pin = recipe
        .manifest()
        .resources
        .iter()
        .find_map(|resource| match resource {
            ResourceDeclaration::Instance(instance) => Some(instance.release),
            ResourceDeclaration::Volume(_) => None,
        })
        .expect("example instance");
    ReleaseCatalogEntry {
        pin,
        published: true,
        parameters: vec![ParameterDeclaration {
            name: ParameterName::parse("database").expect("parameter"),
            value_type: ParameterType::String {
                minimum_length: 1,
                maximum_length: 128,
            },
            required: true,
            default: None,
            sensitive: false,
        }],
        volume_slots: vec![
            VolumeSlotDeclaration::new(
                key("state"),
                GuestMountPath::parse("/data").expect("mount"),
                VolumeAccessMode::ReadWrite,
                true,
                16 * 1024 * 1024,
            )
            .expect("slot"),
        ],
        required_secret_slot_count: 0,
        capability_requirements: vec![
            CapabilityRequirement::new(
                CapabilityRequirementId::from_uuid(uuid::Uuid::from_u128(3)),
                key("state"),
                CapabilityResourceKind::StateVolume,
                [CapabilityOperation::Attach],
                [CapabilityOperation::Inspect],
                true,
            )
            .expect("capability requirement"),
        ],
    }
}

fn assert_code<T>(result: Result<T, RecipeError>, expected: &str) {
    let Err(RecipeError::Invalid(diagnostics)) = result else {
        panic!("expected validation error");
    };
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == expected),
        "{diagnostics:?}"
    );
}

fn parse_manifest(manifest: &crate::RecipeManifest) -> Result<crate::ValidatedRecipe, RecipeError> {
    let text = toml::to_string(manifest).expect("test TOML serialization");
    parse_recipe(text.as_bytes())
}
