use std::collections::BTreeMap;

use capability_domain::{
    CapabilityOperation, CapabilityRequirement, CapabilityRequirementId, CapabilityResourceKind,
};
use release_domain::{ParameterName, ParameterValue};
use runtime_types::{ReleaseId, VolumeId};
use volume_domain::{GuestMountPath, VolumeAccessMode, VolumeSlotDeclaration};

use super::{SAMPLE, assert_code, catalog, key, parse_manifest};
use crate::{
    ExternalVolume, RemovalPolicy, ResolvedResource, ResourceDeclaration, VolumeSource,
    parse_recipe,
};

#[test]
fn resolves_checked_sqlite_example_into_deterministic_intent() {
    let recipe = parse_recipe(SAMPLE.as_bytes()).expect("recipe");
    let resolved = recipe
        .resolve(&BTreeMap::new(), &BTreeMap::new(), &[catalog()])
        .expect("resolve");
    assert_eq!(resolved.execution_order(), &[key("data"), key("app")]);
    let ResolvedResource::Volume(volume) = &resolved.resources()[&key("data")] else {
        panic!("volume")
    };
    assert_eq!(volume.capacity_bytes, 16 * 1024 * 1024);
    assert_eq!(volume.removal, RemovalPolicy::Retain);
    assert_eq!(volume.external_id, None);
    let ResolvedResource::Instance(instance) = &resolved.resources()[&key("app")] else {
        panic!("instance")
    };
    assert_eq!(instance.capability_requirements.len(), 1);
    assert_eq!(
        instance.parameters[&ParameterName::parse("database").expect("parameter")],
        ParameterValue::String("/data/app.sqlite".to_owned())
    );
    let repeated = recipe
        .resolve(&BTreeMap::new(), &BTreeMap::new(), &[catalog()])
        .expect("resolve");
    assert_eq!(
        resolved.hash().expect("hash"),
        repeated.hash().expect("hash")
    );
    assert_eq!(
        resolved.canonical_json().expect("snapshot"),
        repeated.canonical_json().expect("snapshot")
    );
}

#[test]
fn rejects_unknown_missing_wrong_type_and_out_of_range_inputs() {
    let recipe = parse_recipe(SAMPLE.as_bytes()).expect("recipe");
    for (name, value) in [
        ("unknown", ParameterValue::Boolean(true)),
        ("capacity", ParameterValue::Boolean(true)),
        ("capacity", ParameterValue::Integer(1)),
    ] {
        let supplied = BTreeMap::from([(ParameterName::parse(name).expect("parameter"), value)]);
        assert_code(
            recipe.resolve(&supplied, &BTreeMap::new(), &[catalog()]),
            "invalid_supplied_inputs",
        );
    }
    let no_default =
        parse_recipe(SAMPLE.replace("default = 16777216", "").as_bytes()).expect("recipe");
    assert_code(
        no_default.resolve(&BTreeMap::new(), &BTreeMap::new(), &[catalog()]),
        "invalid_supplied_inputs",
    );
}

#[test]
fn exact_release_pins_require_authoritative_publication_and_association() {
    let recipe = parse_recipe(SAMPLE.as_bytes()).expect("recipe");
    assert_code(
        recipe.resolve(&BTreeMap::new(), &BTreeMap::new(), &[]),
        "release_pin_not_in_catalog",
    );
    let mut entry = catalog();
    entry.published = false;
    assert_code(
        recipe.resolve(&BTreeMap::new(), &BTreeMap::new(), &[entry]),
        "release_not_published",
    );
    let mut entry = catalog();
    entry.pin.release_id = ReleaseId::from_uuid(uuid::Uuid::from_u128(10));
    assert_code(
        recipe.resolve(&BTreeMap::new(), &BTreeMap::new(), &[entry]),
        "release_pin_not_in_catalog",
    );
    assert_code(
        recipe.resolve(&BTreeMap::new(), &BTreeMap::new(), &[catalog(), catalog()]),
        "duplicate_catalog_export",
    );
}

#[test]
fn release_volume_slots_bound_mount_mode_capacity_and_requiredness() {
    let recipe = parse_recipe(SAMPLE.as_bytes()).expect("recipe");
    for (path, mode, capacity, code) in [
        (
            "/elsewhere",
            VolumeAccessMode::ReadWrite,
            1,
            "volume_mount_mismatch",
        ),
        (
            "/data",
            VolumeAccessMode::ReadOnly,
            1,
            "volume_access_mismatch",
        ),
        (
            "/data",
            VolumeAccessMode::ReadWrite,
            32 * 1024 * 1024,
            "volume_capacity_below_release_minimum",
        ),
    ] {
        let mut entry = catalog();
        entry.volume_slots = vec![
            VolumeSlotDeclaration::new(
                key("state"),
                GuestMountPath::parse(path).expect("path"),
                mode,
                true,
                capacity,
            )
            .expect("slot"),
        ];
        assert_code(
            recipe.resolve(&BTreeMap::new(), &BTreeMap::new(), &[entry]),
            code,
        );
    }
    let mut manifest = recipe.manifest().clone();
    let ResourceDeclaration::Instance(instance) = &mut manifest.resources[0] else {
        panic!("instance")
    };
    instance.volume_bindings.clear();
    let missing = parse_manifest(&manifest).expect("static intent");
    assert_code(
        missing.resolve(&BTreeMap::new(), &BTreeMap::new(), &[catalog()]),
        "required_volume_slot_missing",
    );
    let mut entry = catalog();
    entry.volume_slots.clear();
    assert_code(
        recipe.resolve(&BTreeMap::new(), &BTreeMap::new(), &[entry]),
        "undeclared_volume_slot",
    );
}

#[test]
fn external_resources_are_explicit_exact_and_never_deleted() {
    let recipe = parse_recipe(SAMPLE.as_bytes()).expect("recipe");
    let mut manifest = recipe.manifest().clone();
    let ResourceDeclaration::Volume(volume) = &mut manifest.resources[1] else {
        panic!("volume")
    };
    volume.source = VolumeSource::External {};
    let external_recipe = parse_manifest(&manifest).expect("external recipe");
    assert_code(
        external_recipe.resolve(&BTreeMap::new(), &BTreeMap::new(), &[catalog()]),
        "external_binding_missing",
    );
    let external = BTreeMap::from([(
        key("data"),
        ExternalVolume {
            id: VolumeId::from_uuid(uuid::Uuid::from_u128(20)),
            capacity_bytes: 16 * 1024 * 1024,
        },
    )]);
    let resolved = external_recipe
        .resolve(&BTreeMap::new(), &external, &[catalog()])
        .expect("external resolved");
    let ResolvedResource::Volume(volume) = &resolved.resources()[&key("data")] else {
        panic!("volume")
    };
    assert_eq!(volume.external_id, Some(external[&key("data")].id));
    assert_eq!(volume.removal, RemovalPolicy::Retain);
    assert_code(
        recipe.resolve(&BTreeMap::new(), &external, &[catalog()]),
        "undeclared_external_binding",
    );
}

#[test]
fn unsupported_required_authority_and_sensitive_parameters_fail_closed() {
    let recipe = parse_recipe(SAMPLE.as_bytes()).expect("recipe");
    let mut entry = catalog();
    entry.capability_requirements.push(
        CapabilityRequirement::new(
            CapabilityRequirementId::from_uuid(uuid::Uuid::from_u128(30)),
            key("repository"),
            CapabilityResourceKind::Repository,
            [CapabilityOperation::GitRead],
            [],
            true,
        )
        .expect("requirement"),
    );
    assert_code(
        recipe.resolve(&BTreeMap::new(), &BTreeMap::new(), &[entry]),
        "unsupported_required_capability_slot",
    );
    let mut entry = catalog();
    entry.parameters[0].sensitive = true;
    assert_code(
        recipe.resolve(&BTreeMap::new(), &BTreeMap::new(), &[entry]),
        "unsupported_release_parameters",
    );
    let mut entry = catalog();
    entry.required_secret_slot_count = 1;
    entry.volume_slots.clear();
    assert_code(
        recipe.resolve(&BTreeMap::new(), &BTreeMap::new(), &[entry]),
        "unsupported_required_secret_slots",
    );
}

#[test]
fn snapshot_hash_captures_authoritative_slot_scope_and_input_values() {
    let recipe = parse_recipe(SAMPLE.as_bytes()).expect("recipe");
    let original = recipe
        .resolve(&BTreeMap::new(), &BTreeMap::new(), &[catalog()])
        .expect("resolve");
    let mut entry = catalog();
    entry.volume_slots = vec![
        VolumeSlotDeclaration::new(
            key("state"),
            GuestMountPath::parse("/data").expect("path"),
            VolumeAccessMode::ReadWrite,
            false,
            1,
        )
        .expect("slot"),
    ];
    let different_scope = recipe
        .resolve(&BTreeMap::new(), &BTreeMap::new(), &[entry])
        .expect("resolve");
    assert_ne!(
        original.hash().expect("hash"),
        different_scope.hash().expect("hash")
    );
    let supplied = BTreeMap::from([(
        ParameterName::parse("capacity").expect("name"),
        ParameterValue::Integer(33_554_432),
    )]);
    let different_input = recipe
        .resolve(&supplied, &BTreeMap::new(), &[catalog()])
        .expect("resolve");
    assert_ne!(
        original.hash().expect("hash"),
        different_input.hash().expect("hash")
    );
    let ResolvedResource::Instance(instance) = &original.resources()[&key("app")] else {
        panic!("instance")
    };
    assert_eq!(instance.dependencies, vec![key("data")]);
    assert_eq!(instance.volume_slots, catalog().volume_slots);
    assert_eq!(original.declaration_hash(), recipe.hash());
    assert_eq!(original.contract_version(), 1);
    assert_eq!(original.recipe_id(), &recipe.manifest().recipe_id);
    assert_eq!(original.recipe_version(), &recipe.manifest().recipe_version);
}

#[test]
fn rejects_external_identity_aliases_and_multiple_writer_instances() {
    let recipe = parse_recipe(SAMPLE.as_bytes()).expect("recipe");
    let mut manifest = recipe.manifest().clone();
    let mut second = manifest.resources[0].clone();
    let ResourceDeclaration::Instance(instance) = &mut second else {
        panic!("instance")
    };
    instance.name = key("second");
    manifest.resources.push(second);
    assert_code(parse_manifest(&manifest), "multiple_volume_writers");
    let mut manifest = recipe.manifest().clone();
    let ResourceDeclaration::Volume(volume) = &mut manifest.resources[1] else {
        panic!("volume")
    };
    volume.source = VolumeSource::External {};
    let mut extra = volume.clone();
    extra.name = key("extra");
    manifest.resources.push(ResourceDeclaration::Volume(extra));
    let external_recipe = parse_manifest(&manifest).expect("external recipe");
    let binding = ExternalVolume {
        id: VolumeId::from_uuid(uuid::Uuid::from_u128(20)),
        capacity_bytes: 16_777_216,
    };
    let external = BTreeMap::from([(key("data"), binding), (key("extra"), binding)]);
    assert_code(
        external_recipe.resolve(&BTreeMap::new(), &external, &[catalog()]),
        "external_resource_alias",
    );
}

#[test]
fn shared_read_only_and_mixed_attachments_are_unsupported() {
    let recipe = parse_recipe(SAMPLE.as_bytes()).expect("recipe");
    for second_mode in [VolumeAccessMode::ReadOnly, VolumeAccessMode::ReadWrite] {
        let mut manifest = recipe.manifest().clone();
        let ResourceDeclaration::Instance(first) = &mut manifest.resources[0] else {
            panic!("instance")
        };
        first.volume_bindings[0].access_mode = VolumeAccessMode::ReadOnly;
        let mut second = first.clone();
        second.name = key("second");
        second.volume_bindings[0].access_mode = second_mode;
        manifest
            .resources
            .push(ResourceDeclaration::Instance(second));
        assert_code(parse_manifest(&manifest), "shared_volume_unsupported");
    }
}
