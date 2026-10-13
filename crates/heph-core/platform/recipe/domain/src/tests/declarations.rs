use super::{SAMPLE, assert_code, key, parse_manifest};
use crate::{
    InputType, MAX_RECIPE_BYTES, RemovalPolicy, ResourceDeclaration, ValueReference, VolumeSource,
    parse_recipe,
};

#[test]
fn rejects_unsupported_versions_and_nested_unknown_fields() {
    assert_code(
        parse_recipe(
            SAMPLE
                .replace("contract_version = 1", "contract_version = 2")
                .as_bytes(),
        ),
        "unsupported_contract_version",
    );
    for (original, replacement) in [
        ("recipe_id =", "unknown = true\nrecipe_id ="),
        ("minimum = 16777216", "unknown = true, minimum = 16777216"),
        (
            "type = \"created\", capacity_bytes = { source = \"input\", name = \"capacity\" }",
            "type = \"external\", host_path = \"/tmp/escape\"",
        ),
        (
            "type = \"integer\", minimum = 16777216, maximum = 1073741824",
            "type = \"boolean\", unknown = true",
        ),
        (
            "source = \"input\", name = \"capacity\"",
            "source = \"input\", name = \"capacity\", expression = \"x + 1\"",
        ),
        (
            "kind = \"volume\"",
            "kind = \"volume\"\nhost_path = \"/tmp/escape\"",
        ),
        (
            "access_mode = \"read_write\"",
            "access_mode = \"read_write\"\npermissions = \"admin\"",
        ),
        ("release_agent_id =", "latest = true, release_agent_id ="),
    ] {
        assert_code(
            parse_recipe(SAMPLE.replace(original, replacement).as_bytes()),
            "invalid_toml",
        );
    }
    assert_code(
        parse_recipe(&vec![b' '; MAX_RECIPE_BYTES + 1]),
        "document_size_limit",
    );
    assert_code(parse_recipe(&[255]), "invalid_utf8");
}

#[test]
fn rejects_duplicates_before_canonicalization() {
    let recipe = parse_recipe(SAMPLE.as_bytes()).expect("recipe");
    let mut manifest = recipe.manifest().clone();
    manifest.resources.push(manifest.resources[0].clone());
    assert_code(parse_manifest(&manifest), "duplicate_resource");
    let mut manifest = recipe.manifest().clone();
    manifest.inputs.push(manifest.inputs[0].clone());
    assert_code(parse_manifest(&manifest), "duplicate_input");
    let mut manifest = recipe.manifest().clone();
    manifest.outputs.push(manifest.outputs[0].clone());
    assert_code(parse_manifest(&manifest), "duplicate_output");
}

#[test]
fn rejects_explicit_and_binding_derived_cycles() {
    let recipe = parse_recipe(SAMPLE.as_bytes()).expect("recipe");
    let mut manifest = recipe.manifest().clone();
    let ResourceDeclaration::Volume(volume) = &mut manifest.resources[1] else {
        panic!("volume")
    };
    volume.depends_on.push(key("app"));
    assert_code(parse_manifest(&manifest), "dependency_cycle");
    volume_reset_and_test_unknown(recipe.manifest().clone());
}

fn volume_reset_and_test_unknown(mut manifest: crate::RecipeManifest) {
    let ResourceDeclaration::Volume(volume) = &mut manifest.resources[1] else {
        panic!("volume")
    };
    volume.depends_on.push(key("absent"));
    assert_code(parse_manifest(&manifest), "unknown_dependency");
}

#[test]
fn rejects_volume_aliases_and_unknown_resources() {
    let recipe = parse_recipe(SAMPLE.as_bytes()).expect("recipe");
    let mut manifest = recipe.manifest().clone();
    let ResourceDeclaration::Instance(instance) = &mut manifest.resources[0] else {
        panic!("instance")
    };
    let mut alias = instance.volume_bindings[0].clone();
    alias.slot = key("second");
    instance.volume_bindings.push(alias);
    assert_code(parse_manifest(&manifest), "volume_alias_reuse");
    let ResourceDeclaration::Instance(instance) = &mut manifest.resources[0] else {
        panic!("instance")
    };
    instance.volume_bindings.pop();
    instance.volume_bindings[0].resource = key("absent");
    assert_code(parse_manifest(&manifest), "unknown_volume_resource");
}

#[test]
fn validates_input_schemas_defaults_capacities_and_external_retention() {
    let recipe = parse_recipe(SAMPLE.as_bytes()).expect("recipe");
    let mut manifest = recipe.manifest().clone();
    manifest.inputs[0].value_type = InputType::Integer {
        minimum: 2,
        maximum: 1,
    };
    assert_code(parse_manifest(&manifest), "invalid_input_bounds");
    let mut manifest = recipe.manifest().clone();
    manifest.inputs[0].default = Some(release_domain::ParameterValue::Boolean(true));
    assert_code(parse_manifest(&manifest), "invalid_input_default");
    let mut manifest = recipe.manifest().clone();
    let ResourceDeclaration::Volume(volume) = &mut manifest.resources[1] else {
        panic!("volume")
    };
    volume.source = VolumeSource::Created {
        capacity_bytes: ValueReference::Literal {
            value: release_domain::ParameterValue::Integer(0),
        },
    };
    assert_code(parse_manifest(&manifest), "invalid_capacity");
    let ResourceDeclaration::Volume(volume) = &mut manifest.resources[1] else {
        panic!("volume")
    };
    volume.source = VolumeSource::External {};
    volume.removal = RemovalPolicy::Delete;
    assert_code(parse_manifest(&manifest), "external_delete_forbidden");
}

#[test]
fn canonical_identity_is_order_independent_and_immutable() {
    let recipe = parse_recipe(SAMPLE.as_bytes()).expect("recipe");
    let mut manifest = recipe.manifest().clone();
    manifest.resources.reverse();
    manifest.inputs.reverse();
    manifest.outputs.reverse();
    let reordered = parse_manifest(&manifest).expect("reordered");
    assert_eq!(recipe.hash(), reordered.hash());
    assert_eq!(recipe.canonical_json(), reordered.canonical_json());
    let changed = parse_recipe(
        SAMPLE
            .replace("default = 16777216", "default = 33554432")
            .as_bytes(),
    )
    .expect("changed recipe");
    assert_code(
        changed.validate_identity(&recipe),
        "recipe_identity_conflict",
    );
}

#[test]
fn rejects_mount_escape_unknown_kind_and_resource_scalar_references() {
    for path in ["/data/../host", "/", "/run", "/data//db"] {
        assert_code(
            parse_recipe(
                SAMPLE
                    .replace(
                        "guest_path = \"/data\"",
                        &format!("guest_path = \"{path}\""),
                    )
                    .as_bytes(),
            ),
            "invalid_toml",
        );
    }
    assert_code(
        parse_recipe(
            SAMPLE
                .replace("kind = \"volume\"", "kind = \"plugin_sql\"")
                .as_bytes(),
        ),
        "invalid_toml",
    );
    assert_code(
        parse_recipe(
            SAMPLE
                .replace(
                    "source = \"input\", name = \"database\"",
                    "source = \"resource\", name = \"data\"",
                )
                .as_bytes(),
        ),
        "invalid_reference",
    );
}

#[test]
fn canonical_source_roundtrips_and_tampered_scope_is_revalidated() {
    let recipe = parse_recipe(SAMPLE.as_bytes()).expect("recipe");
    let canonical = recipe.canonical_toml().expect("canonical TOML");
    let restored = parse_recipe(canonical.as_bytes()).expect("restored declaration");
    assert_eq!(recipe.hash(), restored.hash());
    assert_eq!(
        canonical,
        restored.canonical_toml().expect("canonical TOML")
    );
    let tampered = canonical.replace("guest_path = \"/data\"", "guest_path = \"/run/escape\"");
    assert_code(parse_recipe(tampered.as_bytes()), "invalid_toml");
}
