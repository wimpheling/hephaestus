use std::collections::BTreeMap;

use forge_domain::ProjectId;
use recipe_domain::{
    ExternalVolume, ReleaseCatalogEntry, RemovalPolicy, ResourceDeclaration, parse_recipe,
};
use release_domain::{ParameterName, ParameterValue};
use runtime_types::VolumeId;
use uuid::Uuid;

use super::{VOLUME, deployment, intent, key};
use crate::{
    DeploymentError, DeploymentIntent, DeploymentKey, PlannedResourceIdentity, ResourceAction,
    ResourceOwnership,
};

#[test]
fn canonical_intent_round_trips_and_whitespace_does_not_change_identity() {
    let recipe = parse_recipe(VOLUME.as_bytes()).expect("recipe");
    let original = intent(&recipe, &BTreeMap::new());
    let source = format!("# ordinary declaration comment\n{VOLUME}\n");
    let equivalent = intent(
        &parse_recipe(source.as_bytes()).expect("recipe"),
        &BTreeMap::new(),
    );
    assert_eq!(original.input_hash(), equivalent.input_hash());
    assert_eq!(original.resolved_hash(), equivalent.resolved_hash());
    let restored = parse_recipe(original.declaration_toml().as_bytes()).expect("stored TOML");
    assert_eq!(original, intent(&restored, &BTreeMap::new()));
    assert_eq!(
        original.inputs()[&ParameterName::parse("capacity").expect("name")],
        ParameterValue::Integer(16_777_216)
    );
    assert_eq!(
        original.resolved_json(),
        original.resolved().canonical_json().expect("JSON")
    );
    original
        .validate_replay(&equivalent)
        .expect("unchanged replay");
}

#[test]
fn resolved_inputs_change_fingerprint_without_changing_predicted_resource_identity() {
    let recipe = parse_recipe(VOLUME.as_bytes()).expect("recipe");
    let first = intent(&recipe, &BTreeMap::new());
    let supplied = BTreeMap::from([(
        ParameterName::parse("capacity").expect("input"),
        ParameterValue::Integer(33_554_432),
    )]);
    let changed = intent(&recipe, &supplied);
    assert_eq!(first.declaration_hash(), changed.declaration_hash());
    assert_ne!(first.resolved_hash(), changed.resolved_hash());
    assert_ne!(first.input_hash(), changed.input_hash());
    assert_ne!(
        first.resources()[&key("data")].input_hash(),
        changed.resources()[&key("data")].input_hash()
    );
    assert_eq!(
        first.resources()[&key("data")].identity(),
        changed.resources()[&key("data")].identity()
    );
    assert!(matches!(
        changed.validate_replay(&first),
        Err(DeploymentError::InputConflict)
    ));
}

#[test]
fn constructor_rejects_a_graph_resolved_from_different_declaration_identity() {
    let recipe = parse_recipe(VOLUME.as_bytes()).expect("recipe");
    for source in [
        VOLUME.replace("private-volume", "different-volume"),
        VOLUME.replace("1.0.0", "1.0.1"),
        VOLUME.replace("default = 16777216", "default = 33554432"),
    ] {
        let changed = parse_recipe(source.as_bytes()).expect("different recipe");
        let resolved = changed
            .resolve(&BTreeMap::new(), &BTreeMap::new(), &[])
            .expect("resolve");
        assert!(matches!(
            DeploymentIntent::new(
                deployment(1),
                ProjectId::from_uuid(Uuid::from_u128(2)),
                DeploymentKey::parse("database").expect("key"),
                &recipe,
                &resolved,
            ),
            Err(DeploymentError::IntentMismatch)
        ));
    }
}

#[test]
fn predicted_uuid_v5_ids_are_deployment_name_and_purpose_scoped() {
    let recipe = parse_recipe(VOLUME.as_bytes()).expect("recipe");
    let first = intent(&recipe, &BTreeMap::new());
    let PlannedResourceIdentity::Volume {
        id,
        filesystem_uuid: Some(filesystem),
    } = first.resources()[&key("data")].identity()
    else {
        panic!("owned volume")
    };
    assert_eq!(id.as_uuid().get_version_num(), 5);
    assert_eq!(filesystem.get_version_num(), 5);
    assert_ne!(id.as_uuid(), filesystem);
    let resolved = recipe
        .resolve(&BTreeMap::new(), &BTreeMap::new(), &[])
        .expect("resolve");
    let other = DeploymentIntent::new(
        deployment(3),
        first.project_id(),
        first.key().clone(),
        &recipe,
        &resolved,
    )
    .expect("other deployment");
    assert_ne!(
        first.resources()[&key("data")].identity(),
        other.resources()[&key("data")].identity()
    );
    let renamed = parse_recipe(
        VOLUME
            .replace("name = \"data\"", "name = \"other\"")
            .as_bytes(),
    )
    .expect("renamed");
    assert_ne!(
        first.resources()[&key("data")].identity(),
        intent(&renamed, &BTreeMap::new()).resources()[&key("other")].identity()
    );
}

#[test]
fn external_references_keep_identity_and_reject_all_mutating_actions() {
    let source = VOLUME.replace("source = { type = \"created\", capacity_bytes = { source = \"input\", name = \"capacity\" } }", "source = { type = \"external\" }");
    let recipe = parse_recipe(source.as_bytes()).expect("external recipe");
    let external_id = VolumeId::from_uuid(Uuid::from_u128(9));
    let external = BTreeMap::from([(
        key("data"),
        ExternalVolume {
            id: external_id,
            capacity_bytes: 16_777_216,
        },
    )]);
    let resolved = recipe
        .resolve(&BTreeMap::new(), &external, &[])
        .expect("external resolve");
    let planned = DeploymentIntent::new(
        deployment(1),
        ProjectId::from_uuid(Uuid::from_u128(2)),
        DeploymentKey::parse("database").expect("key"),
        &recipe,
        &resolved,
    )
    .expect("intent");
    let resource = &planned.resources()[&key("data")];
    assert_eq!(
        resource.identity(),
        PlannedResourceIdentity::Volume {
            id: external_id,
            filesystem_uuid: None
        }
    );
    assert_eq!(resource.ownership(), ResourceOwnership::External);
    assert_eq!(resource.removal(), RemovalPolicy::Retain);
    resource
        .validate_action(ResourceAction::VerifyExternal)
        .expect("verification");
    for action in [
        ResourceAction::Create,
        ResourceAction::Delete,
        ResourceAction::Drain,
        ResourceAction::Detach,
        ResourceAction::Retain,
    ] {
        assert!(matches!(
            resource.validate_action(action),
            Err(DeploymentError::InvalidAction)
        ));
    }
    let changed_external = BTreeMap::from([(
        key("data"),
        ExternalVolume {
            id: VolumeId::from_uuid(Uuid::from_u128(10)),
            capacity_bytes: 16_777_216,
        },
    )]);
    let changed_resolved = recipe
        .resolve(&BTreeMap::new(), &changed_external, &[])
        .expect("changed resolve");
    let changed = DeploymentIntent::new(
        planned.id(),
        planned.project_id(),
        planned.key().clone(),
        &recipe,
        &changed_resolved,
    )
    .expect("changed intent");
    assert!(matches!(
        changed.validate_replay(&planned),
        Err(DeploymentError::InputConflict)
    ));
}

#[test]
fn owned_retention_does_not_allow_delete_and_deletion_is_explicit() {
    let recipe = parse_recipe(VOLUME.as_bytes()).expect("recipe");
    let retained = intent(&recipe, &BTreeMap::new());
    let resource = &retained.resources()[&key("data")];
    resource
        .validate_action(ResourceAction::Retain)
        .expect("retain");
    assert!(resource.validate_action(ResourceAction::Delete).is_err());
    assert!(resource.validate_action(ResourceAction::Drain).is_err());
    let source = VOLUME.replace(
        "kind = \"volume\"",
        "kind = \"volume\"\nremoval = \"delete\"",
    );
    let deleted = intent(
        &parse_recipe(source.as_bytes()).expect("recipe"),
        &BTreeMap::new(),
    );
    deleted.resources()[&key("data")]
        .validate_action(ResourceAction::Delete)
        .expect("explicit delete intent");
    assert_ne!(retained.input_hash(), deleted.input_hash());
}

#[test]
fn instance_and_revision_ids_are_distinct_and_secret_requirements_block_resolution() {
    let source = r#"
contract_version = 1
recipe_id = "application"
recipe_version = "1.0.0"
[[resources]]
kind = "instance"
name = "app"
removal = "delete"
release = { release_id = "10000000-0000-4000-8000-000000000001", release_agent_id = "10000000-0000-4000-8000-000000000002" }
"#;
    let recipe = parse_recipe(source.as_bytes()).expect("instance recipe");
    let ResourceDeclaration::Instance(instance) = &recipe.manifest().resources[0] else {
        panic!("instance declaration")
    };
    let mut catalog = ReleaseCatalogEntry {
        pin: instance.release,
        published: true,
        parameters: vec![],
        volume_slots: vec![],
        required_secret_slot_count: 0,
        capability_requirements: vec![],
    };
    let resolved = recipe
        .resolve(&BTreeMap::new(), &BTreeMap::new(), &[catalog.clone()])
        .expect("resolve instance");
    let planned = DeploymentIntent::new(
        deployment(1),
        ProjectId::from_uuid(Uuid::from_u128(2)),
        DeploymentKey::parse("application").expect("key"),
        &recipe,
        &resolved,
    )
    .expect("intent");
    let resource = &planned.resources()[&key("app")];
    let PlannedResourceIdentity::Instance { id, revision_id } = resource.identity() else {
        panic!("instance plan")
    };
    assert_ne!(id.as_uuid(), revision_id.as_uuid());
    assert_eq!(id.as_uuid().get_version_num(), 5);
    assert_eq!(revision_id.as_uuid().get_version_num(), 5);
    resource
        .validate_action(ResourceAction::Drain)
        .expect("owned consumer drain");
    catalog.required_secret_slot_count = 1;
    assert!(
        recipe
            .resolve(&BTreeMap::new(), &BTreeMap::new(), &[catalog])
            .is_err()
    );
}
