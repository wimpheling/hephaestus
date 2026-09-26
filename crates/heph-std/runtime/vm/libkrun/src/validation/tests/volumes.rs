use super::support::{Fixture, assert_invalid_field};
use crate::validation::prepare_spec;
use std::fs;
use vm_trait::{DiskFormat, VmDisk};

#[test]
fn state_volume_labels_require_uuid_mount_and_writable_disk() {
    let fixture = Fixture::new();
    let mut missing_disk = fixture.spec();
    missing_disk.labels.insert(
        String::from("hephaestus.agent-state.filesystem-uuid"),
        uuid::Uuid::new_v4().to_string(),
    );
    missing_disk.labels.insert(
        String::from("hephaestus.agent-state.mount-path"),
        String::from("/var/lib/hephaestus"),
    );
    assert_invalid_field(
        prepare_spec(&fixture.valid_config(), &missing_disk),
        "disks",
    );

    let mut invalid_uuid = missing_disk;
    fs::write(fixture.disks.join("data.raw"), []).unwrap();
    invalid_uuid.disks.push(VmDisk {
        id: String::from("instance-state"),
        host_path: fixture.disks.join("data.raw"),
        format: DiskFormat::Raw,
        read_only: false,
    });
    invalid_uuid.labels.insert(
        String::from("hephaestus.agent-state.filesystem-uuid"),
        String::from("not-a-uuid"),
    );
    assert_invalid_field(
        prepare_spec(&fixture.valid_config(), &invalid_uuid),
        "labels.hephaestus.agent-state.filesystem-uuid",
    );
}

#[test]
fn oci_scratch_volume_is_an_isolated_writable_disk() {
    let fixture = Fixture::new();
    let scratch_path = fixture.disks.join("repository-oci-scratch.raw");
    fs::write(&scratch_path, []).unwrap();

    let mut valid = fixture.spec();
    valid.disks.push(VmDisk {
        id: String::from("repository-oci-scratch"),
        host_path: scratch_path,
        format: DiskFormat::Raw,
        read_only: false,
    });
    valid.labels.insert(
        String::from("hephaestus.oci-scratch.filesystem-uuid"),
        uuid::Uuid::new_v4().to_string(),
    );
    valid.labels.insert(
        String::from("hephaestus.oci-scratch.mount-path"),
        String::from("/workspace/buildah"),
    );
    prepare_spec(&fixture.valid_config(), &valid).unwrap();

    valid.labels.insert(
        String::from("hephaestus.agent-state.filesystem-uuid"),
        uuid::Uuid::new_v4().to_string(),
    );
    valid.labels.insert(
        String::from("hephaestus.agent-state.mount-path"),
        String::from("/var/lib/hephaestus"),
    );
    assert_invalid_field(prepare_spec(&fixture.valid_config(), &valid), "labels");
}

#[test]
fn resource_and_writable_disk_limits_are_rejected() {
    let fixture = Fixture::new();
    let mut cpu = fixture.spec();
    cpu.resources.vcpus = 9;
    assert_invalid_field(prepare_spec(&fixture.config, &cpu), "resources.vcpus");

    let mut memory = fixture.spec();
    memory.resources.memory_mib = 4096;
    assert_invalid_field(
        prepare_spec(&fixture.config, &memory),
        "resources.memory_mib",
    );

    let disk = fixture.disks.join("oversized.raw");
    fs::write(&disk, [0_u8; 32]).unwrap();
    let mut storage = fixture.spec();
    let mut config = fixture.config;
    config.limits.writable_disk_max_bytes = 16;
    storage.disks.push(VmDisk {
        id: String::from("oversized"),
        host_path: disk,
        format: DiskFormat::Raw,
        read_only: false,
    });
    assert_invalid_field(prepare_spec(&config, &storage), "disks");
}
