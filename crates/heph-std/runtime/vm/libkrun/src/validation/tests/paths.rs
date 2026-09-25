use super::support::{Fixture, assert_invalid_field};
use crate::validation::prepare_spec;
use std::{
    fs,
    net::{IpAddr, Ipv4Addr},
    os::unix::fs::symlink,
    path::PathBuf,
};
use vm_trait::{
    DiskFormat, NetworkMode, PortForward, PortProtocol, RootFilesystem, VmDisk, VmMount,
};

#[test]
fn valid_disk_and_mount_paths_are_preserved() {
    let fixture = Fixture::new();
    let disk = fixture.disks.join("sqlite.raw");
    fs::write(&disk, [0_u8; 16]).unwrap();
    let repository = fixture.mounts.join("repository");
    fs::create_dir(&repository).unwrap();
    let mut spec = fixture.spec();
    spec.disks.push(VmDisk {
        id: String::from("sqlite"),
        host_path: disk.clone(),
        format: DiskFormat::Raw,
        read_only: false,
    });
    spec.mounts.push(VmMount {
        tag: String::from("repository"),
        host_path: repository.clone(),
        guest_path: PathBuf::from("/repository"),
        read_only: true,
    });

    let prepared = prepare_spec(&fixture.config, &spec).unwrap();
    assert_eq!(prepared.disks[0].path, disk);
    assert_eq!(prepared.mounts[0].host_path, repository);
}

#[test]
fn symlink_and_parent_traversal_escapes_are_rejected() {
    let fixture = Fixture::new();
    let outside = fixture.temp.path().join("outside.raw");
    fs::write(&outside, [0_u8; 1]).unwrap();
    let link = fixture.disks.join("escape.raw");
    symlink(&outside, &link).unwrap();
    let mut linked = fixture.spec();
    linked.disks.push(VmDisk {
        id: String::from("linked"),
        host_path: link,
        format: DiskFormat::Raw,
        read_only: true,
    });
    assert_invalid_field(prepare_spec(&fixture.config, &linked), "disks[0].host_path");

    let mut traversed = fixture.spec();
    traversed.disks.push(VmDisk {
        id: String::from("traversed"),
        host_path: fixture.disks.join("..").join("outside.raw"),
        format: DiskFormat::Raw,
        read_only: true,
    });
    assert_invalid_field(
        prepare_spec(&fixture.config, &traversed),
        "disks[0].host_path",
    );
}

#[test]
fn missing_paths_and_wrong_file_types_are_rejected() {
    let fixture = Fixture::new();
    let mut missing = fixture.spec();
    missing.root = RootFilesystem::Directory {
        host_path: fixture.images.join("missing"),
    };
    assert_invalid_field(prepare_spec(&fixture.config, &missing), "root.host_path");

    let directory = fixture.disks.join("not-a-disk");
    fs::create_dir(&directory).unwrap();
    let mut wrong_disk = fixture.spec();
    wrong_disk.disks.push(VmDisk {
        id: String::from("wrong"),
        host_path: directory,
        format: DiskFormat::Raw,
        read_only: true,
    });
    assert_invalid_field(
        prepare_spec(&fixture.config, &wrong_disk),
        "disks[0].host_path",
    );

    let file = fixture.mounts.join("not-a-mount");
    fs::write(&file, []).unwrap();
    let mut wrong_mount = fixture.spec();
    wrong_mount.mounts.push(VmMount {
        tag: String::from("wrong"),
        host_path: file,
        guest_path: PathBuf::from("/wrong"),
        read_only: true,
    });
    assert_invalid_field(
        prepare_spec(&fixture.config, &wrong_mount),
        "mounts[0].host_path",
    );
}

#[test]
fn duplicates_and_nul_arguments_are_rejected() {
    let fixture = Fixture::new();
    let mut argument = fixture.spec();
    argument.command.args.push(String::from("bad\0argument"));
    assert_invalid_field(prepare_spec(&fixture.config, &argument), "command.args[0]");

    let mut forwarding = fixture.spec();
    let duplicate = PortForward {
        protocol: PortProtocol::Tcp,
        bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
        host_port: 18080,
        guest_port: 80,
    };
    forwarding.network = NetworkMode::UserMode {
        ingress: vec![duplicate.clone(), duplicate],
    };
    assert_invalid_field(
        prepare_spec(&fixture.config, &forwarding),
        "network.ingress[1]",
    );
}
