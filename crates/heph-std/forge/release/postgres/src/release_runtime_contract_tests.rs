use super::{build, require_legacy_import};
use crate::ReleaseServiceError;
use capability_domain::CapabilitySlotKey;
use release_domain::{
    GuestMountPath, NetworkAccess, RuntimePolicy, VolumeAccessMode, VolumeSlotDeclaration,
};
use serde_json::json;

mod fixture {
    include!("../../../../../heph-core/platform/agent-config/src/tests/support.rs");
}

fn config() -> agent_config::AgentConfig {
    agent_config::parse(fixture::VALID.as_bytes())
        .config
        .expect("legacy fixture")
}

fn slot(name: &str, path: &str) -> VolumeSlotDeclaration {
    VolumeSlotDeclaration::new(
        CapabilitySlotKey::parse(name).expect("slot"),
        GuestMountPath::parse(path).expect("path"),
        VolumeAccessMode::ReadWrite,
        true,
        16 * 1024 * 1024,
    )
    .expect("declaration")
}

fn policy() -> RuntimePolicy {
    RuntimePolicy {
        vcpus: 2,
        memory_mib: 512,
        network: NetworkAccess::Disabled,
    }
}

fn image() -> String {
    format!("registry.example/reviewer@sha256:{}", "0".repeat(64))
}

#[test]
fn legacy_runtime_contract_retains_exact_prechange_bytes_and_hash() {
    let (contract, hash) = build(&config(), &image(), &policy()).expect("legacy contract");
    let expected = r#"{"arguments":["--format=json"],"executable":"bin/review","image_reference":"registry.example/reviewer@sha256:0000000000000000000000000000000000000000000000000000000000000000","policy_ceiling":{"memory_mib":512,"network":"disabled","vcpus":2},"publication_mode":"proposal","publication_repository_slot":null,"requires_state":true,"working_directory":"bin","workspace":{"parameters":"/run/hephaestus/parameters.json","release":"/release","source":"/workspace/repo","state":"/var/lib/hephaestus","work":"/workspace/work"}}"#;
    assert_eq!(
        serde_json::to_vec(&contract).expect("JSON"),
        expected.as_bytes()
    );
    let expected_hash = "70a19c14f4dab9049677309a7add764c6dec8447049449109020f776b6a07910";
    let expected_digest: [u8; 32] = std::array::from_fn(|index| {
        u8::from_str_radix(&expected_hash[index * 2..index * 2 + 2], 16).expect("captured hash")
    });
    assert_eq!(hash, expected_digest);
    assert!(contract.get("volume_slots").is_none());
}

#[test]
fn explicit_authored_slots_are_frozen_in_canonical_order() {
    let legacy_hash = build(&config(), &image(), &policy()).expect("legacy").1;
    let mut config = config();
    config.volume_slots = vec![slot("data", "/data"), slot("cache", "/cache")];
    let (first, first_hash) = build(&config, &image(), &policy()).expect("explicit contract");
    config.volume_slots.reverse();
    let (second, second_hash) = build(&config, &image(), &policy()).expect("reordered contract");
    assert_eq!(first, second);
    assert_eq!(first_hash, second_hash);
    let authored: Vec<VolumeSlotDeclaration> =
        serde_json::from_value(first["volume_slots"].clone()).expect("typed authored slots");
    assert_eq!(
        authored
            .iter()
            .map(|slot| slot.slot().as_str())
            .collect::<Vec<_>>(),
        vec!["cache", "data"]
    );
    assert_eq!(
        authored.len(),
        2,
        "legacy state is not persisted as authored intent"
    );
    assert_ne!(first_hash, legacy_hash);
}

#[test]
fn legacy_import_rejects_explicit_or_malformed_slots_before_metadata_effects() {
    require_legacy_import(&json!({}), true).expect("legacy import");
    require_legacy_import(&json!({"volume_slots": []}), false).expect("empty authored import");
    let explicit = json!({"volume_slots": [slot("data", "/data")]});
    assert!(matches!(
        require_legacy_import(&explicit, true),
        Err(ReleaseServiceError::CapabilityResourceUnavailable)
    ));
    for malformed in [
        json!({"volume_slots": null}),
        json!({"volume_slots": {}}),
        json!({"volume_slots": [{"slot":"data", "guest_path":"/run/escape", "access_mode":"read_write", "required":true,"minimum_capacity_bytes":1}]}),
    ] {
        assert!(matches!(
            require_legacy_import(&malformed, false),
            Err(ReleaseServiceError::InvalidStoredData)
        ));
    }
}
