use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::fixture::{self, SOURCE};
use crate::{DeploymentExecutionPreparation, DeploymentExecutionProfile};

#[test]
fn oversized_complete_preparation_is_rejected_without_truncation() {
    let mut catalog = fixture::catalog();
    let parameter = catalog.parameters[0].clone();
    catalog.parameters = (0..64)
        .map(|index| {
            let mut value = parameter.clone();
            value.name = release_domain::ParameterName::parse(format!("field{index}")).unwrap();
            value.default = Some(release_domain::ParameterValue::String("x".repeat(4096)));
            value
        })
        .collect();
    let intent = fixture::with_catalog(SOURCE, 1, catalog, &BTreeMap::new());
    // Two imports can each legitimately resolve 256 KiB of values. Repeating
    // enough graph instances exceeds the explicit complete-record bound.
    let mut source = SOURCE.to_owned();
    let instance = SOURCE
        .split("[[resources]]\nkind = \"instance\"")
        .nth(1)
        .unwrap();
    for index in 0..4 {
        write!(source,
            "[[resources]]\nkind = \"volume\"\nname = \"extra_data{index}\"\nsource = {{ type = \"created\", capacity_bytes = {{ source = \"literal\", value = 16777216 }} }}\n"
        ).unwrap();
        write!(
            source,
            "[[resources]]\nkind = \"instance\"{}",
            instance
                .replace("name = \"writer\"", &format!("name = \"extra{index}\""))
                .replace(
                    "resource = \"data\"",
                    &format!("resource = \"extra_data{index}\"")
                )
        )
        .unwrap();
    }
    let large = fixture::with_catalog(
        &source,
        1,
        {
            let mut catalog = fixture::catalog();
            catalog.parameters = match intent.resources()
                [&capability_domain::CapabilitySlotKey::parse("writer").unwrap()]
                .intent()
            {
                recipe_domain::ResolvedResource::Instance(instance) => instance
                    .parameters
                    .iter()
                    .map(|(name, value)| release_domain::ParameterDeclaration {
                        name: name.clone(),
                        value_type: parameter.value_type.clone(),
                        required: true,
                        default: Some(value.clone()),
                        sensitive: false,
                    })
                    .collect(),
                recipe_domain::ResolvedResource::Volume(_) => panic!("instance"),
            };
            catalog
        },
        &BTreeMap::new(),
    );
    assert!(
        DeploymentExecutionPreparation::new(
            &large,
            fixture::command(),
            DeploymentExecutionProfile::RuntimeNamedV1,
            fixture::platform("platform/v1"),
            &[fixture::observation()]
        )
        .is_err()
    );
}

#[test]
fn external_binding_preserves_reference_and_only_predicts_a_grant() {
    let source = SOURCE.replacen("source = { type = \"created\", capacity_bytes = { source = \"literal\", value = 16777216 } }", "source = { type = \"external\" }", 1);
    let id = runtime_types::VolumeId::from_uuid(uuid::Uuid::from_u128(50));
    let external = BTreeMap::from([(
        capability_domain::CapabilitySlotKey::parse("data").unwrap(),
        recipe_domain::ExternalVolume {
            id,
            capacity_bytes: 16_777_216,
        },
    )]);
    let intent = fixture::with_catalog(&source, 1, fixture::catalog(), &external);
    let prepared = DeploymentExecutionPreparation::new(
        &intent,
        fixture::command(),
        DeploymentExecutionProfile::RuntimeNamedV1,
        fixture::platform("platform/v1"),
        &[fixture::observation()],
    )
    .unwrap();
    let writer = prepared
        .instances()
        .iter()
        .find(|instance| instance.resource().as_str() == "writer")
        .unwrap();
    assert_eq!(writer.volumes()[0].volume_id(), id);
    let resource =
        &intent.resources()[&capability_domain::CapabilitySlotKey::parse("data").unwrap()];
    assert_eq!(resource.ownership(), crate::ResourceOwnership::External);
    assert!(
        resource
            .validate_action(crate::ResourceAction::Delete)
            .is_err()
    );
}
