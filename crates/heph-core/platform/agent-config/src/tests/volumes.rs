use super::support::VALID;
use crate::parse;
use volume_domain::{MAX_VOLUME_CAPACITY_BYTES, VolumeAccessMode};

fn slot(key: &str, path: &str) -> String {
    format!(
        "\n[[volume_slots]]\nslot = \"{key}\"\nguest_path = \"{path}\"\n\
         access_mode = \"read_write\"\nrequired = true\nminimum_capacity_bytes = 1024\n"
    )
}

#[test]
fn omitted_and_empty_slots_preserve_legacy_normalization_and_authored_configuration() {
    let omitted = parse(VALID.as_bytes());
    assert_eq!(
        omitted
            .normalized_hash
            .as_ref()
            .expect("legacy normalized identity")
            .as_str(),
        "2589b062452446e3ed7306f1bb4ab9c69408438586aa01fab274ad9b983048b5",
        "captured from the legacy fixture before adding named-volume fields"
    );
    let empty = parse(format!("volume_slots = []\n{VALID}").as_bytes());
    assert_eq!(omitted.normalized_hash, empty.normalized_hash);
    let config = omitted.config.expect("legacy configuration");
    assert!(config.volume_slots.is_empty());
    let encoded = serde_json::to_value(&config).expect("serialize configuration");
    assert!(encoded.get("volume_slots").is_none());
    let effective = config.effective_volume_slots().expect("legacy catalog");
    assert_eq!(effective.len(), 1);
    assert_eq!(effective[0].slot().as_str(), "state");
    assert_eq!(effective[0].guest_path().as_str(), "/var/lib/hephaestus");
    assert_eq!(effective[0].access_mode(), VolumeAccessMode::ReadWrite);
    assert_eq!(effective[0].minimum_capacity_bytes(), 1);
    assert!(effective[0].required());
    assert!(config.volume_slots.is_empty());
}

#[test]
fn explicit_slots_are_sorted_for_identity_without_normalizing_in_legacy_state() {
    let alpha = slot("alpha", "/alpha");
    let zeta = slot("zeta", "/zeta");
    let first = parse(format!("{VALID}{zeta}{alpha}").as_bytes());
    let second = parse(format!("{VALID}{alpha}{zeta}").as_bytes());
    assert!(first.diagnostics.is_empty(), "{:?}", first.diagnostics);
    assert_eq!(first.normalized_hash, second.normalized_hash);
    let config = first.config.expect("explicit slots");
    assert_eq!(config.volume_slots[0].slot().as_str(), "zeta");
    assert_eq!(config.volume_slots.len(), 2);
    let effective = config.effective_volume_slots().expect("combined catalog");
    assert_eq!(effective.len(), 3);
    assert_eq!(effective[1].slot().as_str(), "state");
}

#[test]
fn legacy_slot_and_mount_collisions_fail_closed() {
    for declaration in [
        slot("state", "/data"),
        slot("data", "/var/lib/hephaestus"),
        slot("data", "/var/lib/hephaestus/sqlite"),
        slot("data", "/var/lib"),
    ] {
        let parsed = parse(format!("{VALID}{declaration}").as_bytes());
        assert!(parsed.config.is_none());
        assert!(parsed.normalized_hash.is_none());
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "invalid_volume_slots")
        );
    }
    let explicit_state = format!(
        "{}{}",
        VALID.replace("enabled = true", "enabled = false"),
        slot("state", "/var/lib/hephaestus")
    );
    assert!(parse(explicit_state.as_bytes()).config.is_some());
}

#[test]
fn hostile_path_invalid_mode_capacity_and_unknown_nested_fields_are_rejected() {
    let valid = slot("data", "/data");
    let excessive = format!("minimum_capacity_bytes = {}", MAX_VOLUME_CAPACITY_BYTES + 1);
    for invalid in [
        valid.replace("/data", "/data/../escape"),
        valid.replace("/data", "/release/data"),
        valid.replace("/data", "/workspace/data"),
        valid.replace("read_write", "write_only"),
        valid.replace(
            "minimum_capacity_bytes = 1024",
            "minimum_capacity_bytes = 0",
        ),
        valid.replace("minimum_capacity_bytes = 1024", &excessive),
        format!("{valid}host_path = \"/host/data\"\n"),
    ] {
        let parsed = parse(format!("{VALID}{invalid}").as_bytes());
        assert!(parsed.config.is_none());
        assert_eq!(parsed.diagnostics[0].code, "invalid_toml");
    }
}

#[test]
fn explicit_capability_key_collisions_are_rejected_but_historical_state_is_preserved() {
    let capability = "\n[[capability_slots]]\nkey = \"state\"\npurpose = \"Historical binding\"\nresource_kind = \"state_volume\"\nrequired_operations = [\"attach\"]\nrequired = true\n";
    let legacy = parse(format!("{VALID}{capability}").as_bytes());
    assert!(legacy.config.is_some(), "{:?}", legacy.diagnostics);
    let explicit = format!(
        "{}{}{}",
        VALID.replace("enabled = true", "enabled = false"),
        capability,
        slot("state", "/data")
    );
    let parsed = parse(explicit.as_bytes());
    assert!(parsed.config.is_none());
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "invalid_volume_slots")
    );
}

#[test]
fn duplicate_explicit_names_and_mounts_are_rejected() {
    for second in [
        slot("data", "/other"),
        slot("other", "/data"),
        slot("other", "/data/sqlite"),
    ] {
        let parsed = parse(format!("{VALID}{}{second}", slot("data", "/data")).as_bytes());
        assert!(parsed.config.is_none());
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "invalid_volume_slots")
        );
    }
}

#[test]
fn optional_read_only_and_stateless_catalogs_preserve_exact_declarations() {
    let stateless = VALID.replace("enabled = true", "enabled = false");
    assert!(
        parse(stateless.as_bytes())
            .config
            .expect("stateless config")
            .effective_volume_slots()
            .expect("empty catalog")
            .is_empty()
    );
    let optional = slot("data", "/data")
        .replace("read_write", "read_only")
        .replace("required = true", "required = false");
    let config = parse(format!("{stateless}{optional}").as_bytes())
        .config
        .expect("read-only declaration");
    let slots = config.effective_volume_slots().expect("optional catalog");
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].access_mode(), VolumeAccessMode::ReadOnly);
    assert!(!slots[0].required());
    assert_eq!(slots[0].minimum_capacity_bytes(), 1024);
}
