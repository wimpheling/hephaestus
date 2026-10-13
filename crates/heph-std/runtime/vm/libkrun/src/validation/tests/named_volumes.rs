use super::support::Fixture;
use crate::validation::prepare_spec;
use std::fs;
use uuid::Uuid;
use vm_trait::{DiskFormat, GuestMountPath, VmDisk, VmGuestVolume, VolumeAccessMode};

fn named() -> VmGuestVolume {
    VmGuestVolume::new(
        serde_json::from_value(serde_json::json!("data")).unwrap(),
        "data",
        Uuid::from_u128(1),
        GuestMountPath::parse("/data").unwrap(),
        VolumeAccessMode::ReadOnly,
    )
    .unwrap()
}

#[test]
fn exact_named_disk_mode_and_legacy_disambiguation_precede_provisioning() {
    let fixture = Fixture::new();
    let mut spec = fixture.spec();
    spec.guest_volumes = vec![named()];
    assert!(prepare_spec(&fixture.valid_config(), &spec).is_err());
    let disk = fixture.disks.join("data.raw");
    let mut bytes = [0; 1144];
    bytes[1080..1082].copy_from_slice(&[0x53, 0xef]);
    bytes[1128..1144].copy_from_slice(named().filesystem_uuid().as_bytes());
    fs::write(&disk, bytes).unwrap();
    spec.disks.push(VmDisk {
        id: "data".to_owned(),
        host_path: disk,
        format: DiskFormat::Raw,
        read_only: false,
    });
    assert!(prepare_spec(&fixture.valid_config(), &spec).is_err());
    spec.disks.last_mut().unwrap().read_only = true;
    let prepared = prepare_spec(&fixture.valid_config(), &spec).unwrap();
    assert_eq!(prepared.guest_volumes, vec![named()]);
    assert!(prepared.disks.last().unwrap().read_only);
    for key in [
        "hephaestus.agent-state.filesystem-uuid",
        "hephaestus.oci-scratch.mount-path",
    ] {
        spec.labels.insert(key.to_owned(), "ambiguous".to_owned());
        assert!(prepare_spec(&fixture.valid_config(), &spec).is_err());
        spec.labels.remove(key);
    }
}

#[test]
fn exact_disk_uuid_check_rejects_foreign_files_and_final_symlinks() {
    use super::super::{PreparedDisk, validate_named_disk_files};
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let selected = fixture.disks.join("selected.raw");
    let foreign = fixture.disks.join("foreign.raw");
    let mut bytes = [0; 1144];
    bytes[1080..1082].copy_from_slice(&[0x53, 0xef]);
    bytes[1128..1144].copy_from_slice(named().filesystem_uuid().as_bytes());
    fs::write(&foreign, bytes).unwrap();
    let mut disks = vec![
        PreparedDisk {
            id: "data".to_owned(),
            path: selected.clone(),
            read_only: true,
        },
        PreparedDisk {
            id: "foreign".to_owned(),
            path: foreign.clone(),
            read_only: true,
        },
    ];
    bytes[1128..1144].copy_from_slice(Uuid::from_u128(2).as_bytes());
    fs::write(&selected, bytes).unwrap();
    assert!(validate_named_disk_files(&[named()], &disks).is_err());
    fs::remove_file(&selected).unwrap();
    symlink(&foreign, &selected).unwrap();
    assert!(validate_named_disk_files(&[named()], &disks).is_err());
    disks[0].path = foreign;
    validate_named_disk_files(&[named()], &disks).unwrap();
    fs::write(&disks[0].path, [0; 1144]).unwrap();
    assert!(validate_named_disk_files(&[named()], &disks).is_err());
}

#[test]
fn guest_boot_wire_preserves_empty_legacy_shape_and_rejects_aliases() {
    use crate::protocol::{GuestCommandMessage, HostMessage, PROTOCOL_VERSION};
    let message = HostMessage::Start {
        version: PROTOCOL_VERSION,
        command: GuestCommandMessage {
            program: "/bin/true".to_owned(),
            args: vec![],
            env: std::collections::BTreeMap::default(),
            working_dir: None,
        },
        mounts: vec![],
        state_volume: None,
        volumes: vec![],
        runtime_authority: None,
        gateway_handler: false,
        private_http_service: None,
        runtime_git_bridge: None,
    };
    let mut wire = serde_json::to_value(&message).unwrap();
    assert!(wire["Start"].get("volumes").is_none());
    assert!(
        matches!(serde_json::from_value::<HostMessage>(wire.clone()).unwrap(), HostMessage::Start { volumes, .. } if volumes.is_empty())
    );
    wire["Start"]["volumes"] = serde_json::json!([named()]);
    assert!(
        matches!(serde_json::from_value::<HostMessage>(wire.clone()).unwrap(), HostMessage::Start { volumes, state_volume: None, .. } if volumes == vec![named()])
    );
    wire["Start"]["volumes"] = serde_json::json!([named(), named()]);
    assert!(serde_json::from_value::<HostMessage>(wire).is_err());
}

#[test]
fn typed_builtin_wire_requires_frozen_marker_and_survives_roundtrip() {
    let state = VmGuestVolume::new(
        serde_json::from_value(serde_json::json!("state")).unwrap(),
        "state",
        Uuid::from_u128(2),
        GuestMountPath::parse("/var/lib/hephaestus").unwrap(),
        VolumeAccessMode::ReadWrite,
    )
    .unwrap()
    .with_initialization_purpose(
        vm_trait::VmVolumeInitializationPurpose::BuiltinStateSQLite,
        true,
    )
    .unwrap();
    let mut wire = serde_json::to_value(&state).unwrap();
    assert_eq!(
        serde_json::from_value::<VmGuestVolume>(wire.clone()).unwrap(),
        state
    );
    wire["frozen_requires_state"] = serde_json::json!(false);
    assert!(serde_json::from_value::<VmGuestVolume>(wire).is_err());
}
