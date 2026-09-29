use super::support::{Fixture, assert_invalid_field};
use crate::validation::prepare_spec;
use std::{path::PathBuf, time::Duration};
use uuid::Uuid;
use vm_trait::{NetworkMode, PrivateHttpServiceSpec, RuntimeGitBridge, VmError};

#[test]
fn private_http_service_is_prepared_with_bounded_wire_values() {
    let fixture = Fixture::new();
    let mut spec = fixture.spec();
    spec.private_http_service = Some(PrivateHttpServiceSpec {
        loopback_port: 8080,
        max_connections: 4,
        connect_timeout: Duration::from_millis(250),
    });

    let prepared = prepare_spec(&fixture.config, &spec).expect("valid service specification");
    let service = prepared
        .private_http_service
        .expect("prepared service configuration");
    assert_eq!(service.loopback_port, 8080);
    assert_eq!(service.max_connections, 4);
    assert_eq!(service.connect_timeout_ms, 250);
}

#[test]
fn runtime_git_bridge_requires_exact_git_authority_and_private_network() {
    let mut fixture = Fixture::new();
    let repository_id = Uuid::new_v4();
    let mut spec = fixture.spec();
    spec.runtime_git_bridge = Some(RuntimeGitBridge::new(repository_id, 19_100));
    assert_invalid_field(prepare_spec(&fixture.config, &spec), "runtime_git_bridge");

    spec.runtime_authority = Some(
        vm_trait::RuntimeAuthorityBootstrap::new(
            Uuid::new_v4(),
            1,
            [0x11; vm_trait::RUNTIME_AUTHORITY_CREDENTIAL_BYTES],
        )
        .with_runtime_git_credential([0x22; vm_trait::RUNTIME_GIT_CREDENTIAL_BYTES]),
    );
    fixture.config.runtime_git_socket_path =
        Some(PathBuf::from("/run/hephaestus/runtime-git.sock"));
    let prepared = prepare_spec(&fixture.config, &spec).expect("runtime Git bridge");
    let bridge = prepared.runtime_git_bridge.expect("prepared bridge");
    assert_eq!(bridge.repository_id, repository_id);
    assert_eq!(bridge.loopback_port, 19_100);

    spec.network = NetworkMode::UserMode {
        ingress: Vec::new(),
    };
    assert_invalid_field(
        prepare_spec(&fixture.config, &spec),
        "runtime_git_bridge.network",
    );
}

#[test]
fn private_http_service_rejects_invalid_bounds_and_authority() {
    let fixture = Fixture::new();
    let invalid_specs = [
        (
            "port",
            PrivateHttpServiceSpec {
                loopback_port: 80,
                max_connections: 1,
                connect_timeout: Duration::from_secs(1),
            },
        ),
        (
            "connections",
            PrivateHttpServiceSpec {
                loopback_port: 8080,
                max_connections: 65,
                connect_timeout: Duration::from_secs(1),
            },
        ),
        (
            "timeout",
            PrivateHttpServiceSpec {
                loopback_port: 8080,
                max_connections: 1,
                connect_timeout: Duration::from_millis(30_001),
            },
        ),
        (
            "precision",
            PrivateHttpServiceSpec {
                loopback_port: 8080,
                max_connections: 1,
                connect_timeout: Duration::from_nanos(1),
            },
        ),
    ];
    for (expected, service) in invalid_specs {
        let mut spec = fixture.spec();
        spec.private_http_service = Some(service);
        let error = prepare_spec(&fixture.config, &spec).expect_err(expected);
        assert!(matches!(error, VmError::InvalidSpec { .. }), "{expected}");
    }

    let mut authority = fixture.spec();
    authority.private_http_service = Some(PrivateHttpServiceSpec {
        loopback_port: 8080,
        max_connections: 1,
        connect_timeout: Duration::from_secs(1),
    });
    authority.runtime_authority = Some(vm_trait::RuntimeAuthorityBootstrap::new(
        uuid::Uuid::nil(),
        1,
        [0xA5; vm_trait::RUNTIME_AUTHORITY_CREDENTIAL_BYTES],
    ));
    assert_invalid_field(
        prepare_spec(&fixture.config, &authority),
        "runtime_authority",
    );
}

#[test]
fn private_http_service_requires_disabled_network_and_no_handler_label() {
    let fixture = Fixture::new();
    let service = PrivateHttpServiceSpec {
        loopback_port: 8080,
        max_connections: 1,
        connect_timeout: Duration::from_secs(1),
    };

    let mut network = fixture.spec();
    network.private_http_service = Some(service.clone());
    network.network = NetworkMode::UserMode {
        ingress: Vec::new(),
    };
    assert_invalid_field(prepare_spec(&fixture.config, &network), "network");

    let mut handler = fixture.spec();
    handler.private_http_service = Some(service);
    handler.labels.insert(
        crate::protocol::GATEWAY_HANDLER_CONTRACT_LABEL.to_owned(),
        crate::protocol::GATEWAY_HANDLER_CONTRACT_V1.to_owned(),
    );
    assert_invalid_field(
        prepare_spec(&fixture.config, &handler),
        "private_http_service",
    );
}
