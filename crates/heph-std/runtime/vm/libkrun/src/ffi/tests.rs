use super::{KrunApi, context_from_api, deterministic_mac, path_cstring, status};
use crate::validation::{
    PreparedNetwork, PreparedPrivateHttpService, PreparedRoot, PreparedRuntimeGitBridge,
};
use std::{path::Path, sync::Arc};

#[path = "test_support.rs"]
mod support;
use support::{Call, RecordingApi, prepared_spec};

#[test]
fn negative_status_becomes_typed_error() {
    let error = status("test", -22).unwrap_err();
    assert_eq!(error.diagnostic_code(), "libkrun-errno-22");
    assert!(error.to_string().contains("-22"));
}

#[test]
fn ffi_paths_reject_interior_nul() {
    assert!(path_cstring(Path::new("bad\0path")).is_err());
}

#[test]
fn generated_mac_is_local_and_stable() {
    let first = deterministic_mac("agent-1");
    assert_eq!(first, deterministic_mac("agent-1"));
    assert_eq!(first[0] & 0b11, 0b10);
}

#[test]
fn safe_configuration_maps_every_device_to_expected_api_call() {
    let api = Arc::new(RecordingApi::new(None));
    let context = context_from_api(api.clone()).unwrap();
    context
        .configure(
            &prepared_spec(),
            Some(Path::new("/run/vm/passt.sock")),
            Path::new("/run/vm/control.sock"),
            None,
            None,
            None,
        )
        .unwrap();
    drop(context);

    assert_eq!(
        api.calls(),
        vec![
            Call::Create,
            Call::VmConfig {
                id: 7,
                vcpus: 2,
                memory_mib: 1024,
            },
            Call::Root {
                id: 7,
                path: String::from("/images/root"),
            },
            Call::Disk {
                id: 7,
                block_id: String::from("sqlite"),
                path: String::from("/disks/sqlite.raw"),
                read_only: false,
            },
            Call::VirtioFs {
                id: 7,
                tag: String::from("workspace"),
                path: String::from("/mounts/workspace"),
                read_only: false,
            },
            Call::DisableImplicitVsock { id: 7 },
            Call::Vsock { id: 7, cid: 0 },
            Call::VsockPort {
                id: 7,
                port: crate::protocol::GUEST_VSOCK_PORT,
                path: String::from("/run/vm/control.sock"),
            },
            Call::Network {
                id: 7,
                path: String::from("/run/vm/passt.sock"),
                mac: deterministic_mac("recording"),
            },
            Call::Exec {
                id: 7,
                executable: String::from("/usr/libexec/hephaestus/heph-init"),
            },
            Call::Free { id: 7 },
        ]
    );
}

#[test]
fn raw_root_uses_explicit_raw_disk_and_remount_calls() {
    let api = Arc::new(RecordingApi::new(None));
    let context = context_from_api(api.clone()).unwrap();
    let mut spec = prepared_spec();
    spec.root = PreparedRoot::RawDisk {
        path: Path::new("/images/root.raw").to_path_buf(),
        read_only: true,
    };
    spec.disks.clear();
    spec.mounts.clear();
    spec.network = PreparedNetwork::Disabled;
    context
        .configure(
            &spec,
            None,
            Path::new("/run/vm/control.sock"),
            None,
            None,
            None,
        )
        .unwrap();
    drop(context);
    let calls = api.calls();
    assert!(calls.contains(&Call::Disk {
        id: 7,
        block_id: String::from("root"),
        path: String::from("/images/root.raw"),
        read_only: true,
    }));
    assert!(calls.contains(&Call::RootRemount {
        id: 7,
        device: String::from("/dev/vda"),
        filesystem: String::from("auto"),
    }));
    assert!(
        !calls
            .iter()
            .any(|call| matches!(call, Call::Network { .. }))
    );
}

#[test]
fn broker_only_adds_dedicated_vsock_without_ip_network() {
    let api = Arc::new(RecordingApi::new(None));
    let context = context_from_api(api.clone()).unwrap();
    let mut spec = prepared_spec();
    spec.network = PreparedNetwork::BrokerOnly;
    context
        .configure(
            &spec,
            None,
            Path::new("/run/vm/control.sock"),
            Some(Path::new("/run/hephaestus/broker.sock")),
            None,
            None,
        )
        .unwrap();
    drop(context);
    let calls = api.calls();
    assert!(calls.contains(&Call::VsockPort {
        id: 7,
        port: crate::protocol::SECRET_BROKER_VSOCK_PORT,
        path: String::from("/run/hephaestus/broker.sock"),
    }));
    assert!(
        calls
            .iter()
            .all(|call| !matches!(call, Call::Network { .. }))
    );
}

#[test]
fn private_service_maps_only_the_dedicated_vsock() {
    let api = Arc::new(RecordingApi::new(None));
    let context = context_from_api(api.clone()).unwrap();
    let mut spec = prepared_spec();
    spec.network = PreparedNetwork::Disabled;
    spec.private_http_service = Some(PreparedPrivateHttpService {
        loopback_port: 8080,
        max_connections: 4,
        connect_timeout_ms: 1_000,
    });
    context
        .configure(
            &spec,
            None,
            Path::new("/run/vm/control.sock"),
            None,
            Some(Path::new("/run/vm/private-service.sock")),
            None,
        )
        .unwrap();
    drop(context);
    let calls = api.calls();
    assert!(calls.contains(&Call::VsockPort {
        id: 7,
        port: crate::protocol::PRIVATE_SERVICE_VSOCK_PORT,
        path: String::from("/run/vm/private-service.sock"),
    }));
    assert!(
        calls
            .iter()
            .all(|call| !matches!(call, Call::Network { .. }))
    );
}

#[test]
fn runtime_git_maps_a_separate_guest_to_host_vsock() {
    let api = Arc::new(RecordingApi::new(None));
    let context = context_from_api(api.clone()).unwrap();
    let mut spec = prepared_spec();
    spec.network = PreparedNetwork::Disabled;
    spec.private_http_service = None;
    spec.runtime_authority = Some(crate::validation::PreparedRuntimeAuthority {
        session_id: uuid::Uuid::new_v4(),
        generation: 1,
        credential: [0x11; vm_trait::RUNTIME_AUTHORITY_CREDENTIAL_BYTES],
        runtime_git_credential: Some([0x22; vm_trait::RUNTIME_GIT_CREDENTIAL_BYTES]),
    });
    spec.runtime_git_bridge = Some(PreparedRuntimeGitBridge {
        repository_id: uuid::Uuid::new_v4(),
        loopback_port: 19_100,
    });
    context
        .configure(
            &spec,
            None,
            Path::new("/run/vm/control.sock"),
            None,
            None,
            Some(Path::new("/run/hephaestus/runtime-git.sock")),
        )
        .unwrap();
    let calls = api.calls();
    assert!(calls.contains(&Call::VsockPort {
        id: 7,
        port: crate::protocol::RUNTIME_GIT_VSOCK_PORT,
        path: String::from("/run/hephaestus/runtime-git.sock"),
    }));
    assert!(
        calls
            .iter()
            .all(|call| !matches!(call, Call::Network { .. }))
    );
}

#[test]
fn every_injected_api_failure_retains_operation_and_code() {
    for operation in [
        "create",
        "vm-config",
        "root",
        "disk",
        "virtio-fs",
        "disable-vsock",
        "vsock",
        "vsock-port",
        "network",
        "exec",
    ] {
        let api = Arc::new(RecordingApi::new(Some(operation)));
        let result = context_from_api(api.clone()).and_then(|context| {
            context.configure(
                &prepared_spec(),
                Some(Path::new("/run/vm/passt.sock")),
                Path::new("/run/vm/control.sock"),
                None,
                None,
                None,
            )
        });
        let error = result.expect_err("injected FFI failure must propagate");
        assert_eq!(error.diagnostic_code(), "libkrun-errno-22");
        assert!(error.to_string().contains("-22"));
        if operation != "create" {
            assert_eq!(api.calls().last(), Some(&Call::Free { id: 7 }));
        }
    }

    let remount_api = Arc::new(RecordingApi::new(Some("root-remount")));
    let remount_context = context_from_api(remount_api).unwrap();
    let mut raw_root = prepared_spec();
    raw_root.root = PreparedRoot::RawDisk {
        path: Path::new("/images/root.raw").to_path_buf(),
        read_only: true,
    };
    assert_eq!(
        remount_context
            .configure(
                &raw_root,
                Some(Path::new("/run/vm/passt.sock")),
                Path::new("/run/vm/control.sock"),
                None,
                None,
                None,
            )
            .unwrap_err()
            .diagnostic_code(),
        "libkrun-errno-22"
    );

    let start_api = Arc::new(RecordingApi::new(Some("start")));
    let start_context = context_from_api(start_api).unwrap();
    assert_eq!(
        start_context.start_enter().unwrap_err().diagnostic_code(),
        "libkrun-errno-22"
    );
}
