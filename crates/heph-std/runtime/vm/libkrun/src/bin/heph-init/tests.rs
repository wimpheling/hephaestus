use crate::{PlatformOciOperation, find_ext4_device_in, platform_oci_operation};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Seek, SeekFrom, Write},
};
use tempfile::TempDir;
use uuid::Uuid;
use vm_libkrun::protocol::GuestCommandMessage;

#[test]
fn locates_ext4_device_by_filesystem_uuid() {
    let temp = TempDir::new().unwrap();
    let blocks = temp.path().join("blocks");
    let devices = temp.path().join("devices");
    fs::create_dir(&blocks).unwrap();
    fs::create_dir(&devices).unwrap();
    fs::create_dir(blocks.join("vda")).unwrap();
    fs::create_dir(blocks.join("vdb")).unwrap();
    fs::write(devices.join("vda"), vec![0_u8; 2048]).unwrap();
    let expected = Uuid::new_v4();
    let mut device = File::create(devices.join("vdb")).unwrap();
    device.set_len(2048).unwrap();
    device.seek(SeekFrom::Start(1024 + 56)).unwrap();
    device.write_all(&[0x53, 0xef]).unwrap();
    device.seek(SeekFrom::Start(1024 + 104)).unwrap();
    device.write_all(expected.as_bytes()).unwrap();
    drop(device);

    assert_eq!(
        find_ext4_device_in(expected, &blocks, &devices).unwrap(),
        devices.join("vdb")
    );
}

#[test]
fn only_exact_internal_oci_operations_can_use_guest_root() {
    let command = GuestCommandMessage {
        program: String::from("/usr/libexec/hephaestus/oci-build"),
        args: Vec::new(),
        env: BTreeMap::from([(String::from("HEPH_PLATFORM_OCI_BUILDER"), String::from("1"))]),
        working_dir: None,
    };
    assert_eq!(
        platform_oci_operation(&command),
        PlatformOciOperation::Builder
    );
    assert!(platform_oci_operation(&command).runs_as_root());

    let different_command = GuestCommandMessage {
        program: String::from("/bin/sh"),
        ..command
    };
    assert_eq!(
        platform_oci_operation(&different_command),
        PlatformOciOperation::None
    );

    let verifier = GuestCommandMessage {
        program: String::from("/usr/libexec/hephaestus/oci-verify"),
        args: Vec::new(),
        env: BTreeMap::from([(
            String::from("HEPH_PLATFORM_OCI_VERIFIER"),
            String::from("1"),
        )]),
        working_dir: None,
    };
    assert_eq!(
        platform_oci_operation(&verifier),
        PlatformOciOperation::Verifier
    );
    assert!(!platform_oci_operation(&verifier).runs_as_root());
}
