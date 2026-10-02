//! Production-parser checks for the source-only local `SQLite` package.
//!
//! Typed release identities and publication state below are parser fixtures,
//! not evidence of a published release, installed image, or running deployment.

use std::{collections::BTreeMap, path::PathBuf, process::Command};

use agent_config::{AgentConfig, BuildArtifactKind, NetworkProfile, parse};
use capability_domain::CapabilitySlotKey;
use recipe_domain::{
    ExternalVolume, ReleaseCatalogEntry, ReleasePin, RemovalPolicy, ResolvedInstance,
    ResolvedRecipe, ResolvedResource, ValidatedRecipe, parse_recipe,
};
use release_domain::{
    ParameterDeclaration, ParameterName, ParameterValue, ReleaseAgentId, ReleaseId,
};
use serde_json::json;
use uuid::Uuid;
use volume_domain::VolumeAccessMode;

const AGENT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../examples/local-sqlite/agent.toml"
));

fn config() -> AgentConfig {
    let parsed = parse(AGENT);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert!(parsed.normalized_hash.is_some());
    parsed.config.expect("validated actual SQLite agent.toml")
}

fn pin() -> ReleasePin {
    ReleasePin {
        release_id: ReleaseId::from_uuid(
            Uuid::parse_str("3b5025bc-d56c-420c-99de-2dbdf7be96a1").unwrap(),
        ),
        release_agent_id: ReleaseAgentId::from_uuid(
            Uuid::parse_str("d21b6db0-4b5b-4b3b-bbfa-222f91324aec").unwrap(),
        ),
    }
}

fn generated_recipe(pin: ReleasePin, reuse: bool) -> Vec<u8> {
    let generator = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../examples/local-sqlite/render_recipe.py");
    let publication_fixture = json!({"release": {
        "state": "published", "id": {"value": pin.release_id.as_uuid()},
        "agents": [{"agent_key": "local-sqlite", "id": {"value": pin.release_agent_id.as_uuid()}}],
    }});
    // Execute the actual package renderer without saving fabricated publication
    // output beside the example. This is a syntax/compatibility fixture only.
    let output = Command::new("python3")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .args([
            "-c",
            "import json,runpy,sys; print(runpy.run_path(sys.argv[1])['render'](json.loads(sys.argv[2]),reuse=sys.argv[3]=='1'),end='')",
        ])
        .arg(generator)
        .arg(publication_fixture.to_string())
        .arg(if reuse { "1" } else { "0" })
        .output()
        .expect("Python 3 is required to exercise the package renderer");
    assert!(output.status.success(), "{:?}", output.stderr);
    output.stdout
}

fn catalog(config: &AgentConfig, pin: ReleasePin) -> ReleaseCatalogEntry {
    assert!(config.capability_slots.is_empty());
    let parameters = config
        .parameters
        .iter()
        .map(|parameter| ParameterDeclaration {
            name: ParameterName::parse(parameter.name.clone()).unwrap(),
            value_type: serde_json::from_value(
                serde_json::to_value(&parameter.value_type).unwrap(),
            )
            .unwrap(),
            required: parameter.required,
            default: parameter
                .default
                .as_ref()
                .map(|value| serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()),
            sensitive: parameter.sensitive,
        })
        .collect();
    ReleaseCatalogEntry {
        pin,
        published: true, // Parser fixture; production must load server publication facts.
        parameters,
        volume_slots: config.effective_volume_slots().unwrap(),
        required_secret_slot_count: config
            .secret_slots
            .iter()
            .filter(|slot| slot.required)
            .count(),
        capability_requirements: Vec::new(),
    }
}

#[test]
fn actual_sqlite_package_and_generated_recipe_match_released_contract() {
    let config = config();
    assert_package_contract(&config);
    let pin = pin();
    let catalog = catalog(&config, pin);
    let source = generated_recipe(pin, false);
    let recipe = parse_recipe(&source).expect("actual generated TOML validates");
    let resolved = recipe
        .resolve(&BTreeMap::new(), &BTreeMap::new(), &[catalog.clone()])
        .expect("generated graph matches the parsed package declarations");
    assert_eq!(resolved.recipe_id().as_str(), "local-sqlite");
    assert_eq!(resolved.execution_order(), &[slot("data"), slot("app")]);
    let ResolvedResource::Volume(volume) = &resolved.resources()[&slot("data")] else {
        panic!("data must be a volume")
    };
    assert_eq!(volume.capacity_bytes, 16_777_216);
    assert_eq!(volume.removal, RemovalPolicy::Retain);
    assert!(volume.external_id.is_none());
    let ResolvedResource::Instance(instance) = &resolved.resources()[&slot("app")] else {
        panic!("app must be an instance")
    };
    assert_eq!(instance.release, pin);
    assert_eq!(instance.removal, RemovalPolicy::Delete);
    assert_eq!(instance.volume_slots, config.volume_slots);
    assert_eq!(instance.volume_bindings.len(), 1);
    let binding = &instance.volume_bindings[0];
    assert_eq!(binding.slot, slot("data"));
    assert_eq!(binding.resource, slot("data"));
    assert_eq!(binding.guest_path.as_str(), "/data");
    assert_eq!(binding.access_mode, VolumeAccessMode::ReadWrite);
    let defaults = catalog
        .parameters
        .iter()
        .map(|parameter| (parameter.name.clone(), parameter.default.clone().unwrap()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(instance.parameters, defaults);
    assert_eq!(
        instance.parameters[&ParameterName::parse("operation").unwrap()],
        ParameterValue::String("inspect".to_owned())
    );
    assert_eq!(resolved.outputs().len(), 2);
    assert_command_inputs(&recipe, &catalog, &resolved);
    assert_reuse_profile(&catalog, pin);
    println!(
        "LOCAL_SQLITE_PARSER_FIXTURE=1 actual_agent_toml=1 actual_renderer=1 typed_pins=1 parsed_catalog=1 named_rw=1 retained_volume=1 defaults_override_reuse=1 parser_fixture_only=1"
    );
}

fn assert_package_contract(config: &AgentConfig) {
    assert_eq!(config.agent.key.as_deref(), Some("local-sqlite"));
    assert_eq!(config.guest.image.key.as_deref(), Some("python-ubuntu"));
    assert_eq!(config.guest.command, "bin/local-sqlite");
    assert_eq!(config.guest.working_directory, "bin");
    assert!(!config.workspace.mount);
    assert!(!config.state_volume.enabled);
    assert!(!config.triggers.push);
    assert!(matches!(config.network.profile, NetworkProfile::Disabled));
    assert_build_contract(config);
    assert_eq!(config.parameters.len(), 3);
    assert!(config.secret_slots.is_empty());
    assert_eq!(config.volume_slots.len(), 1);
    let declaration = &config.volume_slots[0];
    assert_eq!(declaration.slot(), &slot("data"));
    assert_eq!(declaration.guest_path().as_str(), "/data");
    assert_eq!(declaration.access_mode(), VolumeAccessMode::ReadWrite);
    assert!(declaration.required());
    assert_eq!(declaration.minimum_capacity_bytes(), 16_777_216);
}

fn slot(value: &str) -> CapabilitySlotKey {
    CapabilitySlotKey::parse(value).unwrap()
}

fn assert_build_contract(config: &AgentConfig) {
    let build = config.build.as_ref().unwrap();
    assert_eq!(build.image.key.as_deref(), Some("python-ubuntu"));
    assert_eq!(build.command, "/bin/sh");
    assert_eq!(build.arguments, ["build.sh"]);
    assert_eq!(build.working_directory, "/workspace/source");
    assert!(matches!(build.network.profile, NetworkProfile::Disabled));
    assert_eq!(build.artifacts.len(), 1);
    assert_eq!(build.artifacts[0].path, config.guest.command);
    assert!(matches!(
        build.artifacts[0].kind,
        BuildArtifactKind::Executable
    ));
}

fn command_inputs(
    operation: &str,
    key: &str,
    value: &str,
) -> BTreeMap<ParameterName, ParameterValue> {
    [("operation", operation), ("key", key), ("value", value)]
        .into_iter()
        .map(|(name, value)| {
            (
                ParameterName::parse(name).unwrap(),
                ParameterValue::String(value.to_owned()),
            )
        })
        .collect()
}

fn instance(recipe: &ResolvedRecipe) -> &ResolvedInstance {
    let ResolvedResource::Instance(instance) = &recipe.resources()[&slot("app")] else {
        panic!("app must be an instance")
    };
    instance
}

fn assert_command_inputs(
    recipe: &ValidatedRecipe,
    catalog: &ReleaseCatalogEntry,
    defaults: &ResolvedRecipe,
) {
    let supplied = command_inputs("put", "example", "durable");
    let write = recipe
        .resolve(&supplied, &BTreeMap::new(), std::slice::from_ref(catalog))
        .expect("same rendered definition accepts a bounded put request");
    assert_eq!(instance(&write).parameters, supplied);
    assert_eq!(write.declaration_hash(), defaults.declaration_hash());
    assert_ne!(write.hash().unwrap(), defaults.hash().unwrap());
}

fn assert_reuse_profile(catalog: &ReleaseCatalogEntry, pin: ReleasePin) {
    let recipe = parse_recipe(&generated_recipe(pin, true)).unwrap();
    assert_eq!(recipe.manifest().recipe_id.as_str(), "local-sqlite-reuse");
    let external = ExternalVolume {
        // Exact retained-volume identity is a parser fixture, not a provider handle.
        id: Uuid::parse_str("c34c89b9-0476-4568-a6e6-9251fa4953ec")
            .unwrap()
            .into(),
        capacity_bytes: 16_777_216,
    };
    let bindings = BTreeMap::from([(slot("data"), external)]);
    for inputs in [
        command_inputs("get", "example", ""),
        command_inputs("integrity", "", ""),
    ] {
        let resolved = recipe
            .resolve(&inputs, &bindings, std::slice::from_ref(catalog))
            .expect("reuse profile resolves the explicit external retained-volume fixture");
        assert_eq!(instance(&resolved).parameters, inputs);
        assert_eq!(instance(&resolved).release, pin);
        assert_eq!(instance(&resolved).volume_slots, catalog.volume_slots);
        let ResolvedResource::Volume(volume) = &resolved.resources()[&slot("data")] else {
            panic!("external data volume")
        };
        assert_eq!(volume.external_id, Some(external.id));
        assert_eq!(volume.removal, RemovalPolicy::Retain);
        assert_eq!(resolved.execution_order(), &[slot("data"), slot("app")]);
    }
}
