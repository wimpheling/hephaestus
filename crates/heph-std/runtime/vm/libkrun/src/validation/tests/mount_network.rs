use super::support::{Fixture, assert_invalid_field};
use crate::validation::{PreparedNetwork, prepare_spec};
use std::{
    fs,
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
};
use vm_trait::{
    DiskFormat, NetworkMode, PortForward, PortProtocol, RootFilesystem, VmError, VmMount,
};

#[test]
fn mount_escape_is_rejected() {
    let fixture = Fixture::new();
    let outside = fixture.temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let mut spec = fixture.spec();
    spec.mounts.push(VmMount {
        tag: "repository".to_owned(),
        host_path: outside,
        guest_path: PathBuf::from("/repository"),
        read_only: true,
    });

    assert!(matches!(
        prepare_spec(&fixture.config, &spec),
        Err(VmError::InvalidSpec { field, .. }) if field == "mounts[0].host_path"
    ));
}

#[test]
fn oversized_mount_tag_is_rejected_before_ffi() {
    let fixture = Fixture::new();
    let repository = fixture.mounts.join("repository");
    fs::create_dir(&repository).unwrap();
    let mut spec = fixture.spec();
    spec.mounts.push(VmMount {
        tag: "x".repeat(37),
        host_path: repository,
        guest_path: PathBuf::from("/repository"),
        read_only: true,
    });

    assert!(matches!(
        prepare_spec(&fixture.config, &spec),
        Err(VmError::InvalidSpec { field, .. }) if field == "mounts[0].tag"
    ));
}

#[test]
fn non_raw_root_is_typed_as_unsupported() {
    let fixture = Fixture::new();
    let image = fixture.images.join("root.qcow2");
    fs::write(&image, "image").unwrap();
    let mut spec = fixture.spec();
    spec.root = RootFilesystem::Disk {
        host_path: image,
        format: DiskFormat::Qcow2,
        read_only: true,
    };

    assert!(matches!(
        prepare_spec(&fixture.config, &spec),
        Err(VmError::Unsupported { .. })
    ));
}

#[test]
fn forwarding_requires_tcp_on_ipv4_localhost() {
    let fixture = Fixture::new();
    let mut spec = fixture.spec();
    spec.network = NetworkMode::UserMode {
        ingress: vec![PortForward {
            protocol: PortProtocol::Tcp,
            bind_addr: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            host_port: 0,
            guest_port: 22,
        }],
    };

    assert!(matches!(
        prepare_spec(&fixture.config, &spec),
        Err(VmError::InvalidSpec { field, .. })
            if field == "network.ingress[0].bind_addr"
    ));
}

#[test]
fn broker_only_requires_host_transport_and_has_no_ip_network() {
    let mut fixture = Fixture::new();
    let mut spec = fixture.spec();
    spec.network = NetworkMode::BrokerOnly;
    assert_invalid_field(prepare_spec(&fixture.config, &spec), "network");

    fixture.config.broker_socket_path = Some(PathBuf::from("/run/hephaestus/broker.sock"));
    let prepared = prepare_spec(&fixture.config, &spec).expect("broker transport");
    assert!(matches!(prepared.network, PreparedNetwork::BrokerOnly));
}
