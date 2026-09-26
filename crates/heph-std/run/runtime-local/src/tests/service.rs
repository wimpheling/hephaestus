use super::support::{service_fixture, service_identity};
use crate::{
    GATEWAY_SERVICE_METADATA, GATEWAY_SERVICE_SCHEMA_VERSION, GATEWAY_SERVICE_STAGING_PREFIX,
    GatewayServiceIdentity, MAX_GATEWAY_SERVICE_METADATA_BYTES,
};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    process::Command,
};
use uuid::Uuid;

#[test]
fn service_materialization_seals_identity_and_mounts_only_guest_trees() {
    let (_fixture, runtime, artifact) = service_fixture();
    let identity = service_identity();
    let mounts = runtime
        .prepare_service(identity, &[artifact], &serde_json::json!({"port": 8080}))
        .expect("materialize service");
    assert_eq!(mounts.len(), 2);
    assert!(mounts.iter().all(|mount| mount.read_only));
    assert!(mounts.iter().all(|mount| mount.tag.len() <= 36));
    assert_eq!(mounts[0].guest_path, std::path::PathBuf::from("/release"));
    assert_eq!(
        mounts[1].guest_path,
        std::path::PathBuf::from("/run/hephaestus")
    );
    assert!(
        mounts
            .iter()
            .all(|mount| !mount.host_path.ends_with(GATEWAY_SERVICE_METADATA))
    );

    let active = runtime.service_path(identity.instance_id);
    let identity_path = active.join(GATEWAY_SERVICE_METADATA);
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(&identity_path).expect("identity metadata"))
            .expect("identity JSON");
    assert_eq!(metadata["schema_version"], GATEWAY_SERVICE_SCHEMA_VERSION);
    assert_eq!(metadata["gateway_id"], identity.gateway_id.to_string());
    assert_eq!(
        fs::metadata(identity_path)
            .expect("identity mode")
            .permissions()
            .mode()
            & 0o222,
        0
    );
    assert_eq!(runtime.enumerate_service_instances().unwrap().len(), 1);
}

#[test]
fn service_identity_rejects_writable_oversized_and_nonregular_metadata() {
    let (_fixture, runtime, artifact) = service_fixture();
    let identity = service_identity();
    runtime
        .prepare_service(identity, &[artifact], &serde_json::json!({}))
        .expect("materialize service");
    fs::set_permissions(
        runtime
            .service_path(identity.instance_id)
            .join(GATEWAY_SERVICE_METADATA),
        fs::Permissions::from_mode(0o644),
    )
    .expect("make identity writable");
    assert!(runtime.enumerate_service_instances().is_err());

    let (_fixture, runtime, artifact) = service_fixture();
    let identity = service_identity();
    runtime
        .prepare_service(identity, &[artifact], &serde_json::json!({}))
        .expect("materialize service");
    let active = runtime.service_path(identity.instance_id);
    fs::set_permissions(&active, fs::Permissions::from_mode(0o700)).expect("open active");
    let metadata_path = active.join(GATEWAY_SERVICE_METADATA);
    fs::remove_file(&metadata_path).expect("remove identity");
    fs::write(
        &metadata_path,
        vec![
            b'x';
            usize::try_from(MAX_GATEWAY_SERVICE_METADATA_BYTES)
                .expect("metadata limit fits in usize")
                + 1
        ],
    )
    .expect("write oversized identity");
    fs::set_permissions(&metadata_path, fs::Permissions::from_mode(0o444))
        .expect("seal oversized identity");
    assert!(runtime.enumerate_service_instances().is_err());

    let (_fixture, runtime, artifact) = service_fixture();
    let identity = service_identity();
    runtime
        .prepare_service(identity, &[artifact], &serde_json::json!({}))
        .expect("materialize service");
    let active = runtime.service_path(identity.instance_id);
    fs::set_permissions(&active, fs::Permissions::from_mode(0o700)).expect("open active");
    let metadata_path = active.join(GATEWAY_SERVICE_METADATA);
    fs::remove_file(&metadata_path).expect("remove identity");
    assert!(
        Command::new("mkfifo")
            .arg(&metadata_path)
            .status()
            .expect("mkfifo")
            .success()
    );
    assert!(runtime.enumerate_service_instances().is_err());
}

#[test]
fn service_instances_coexist_and_identity_mismatch_fails_closed() {
    let (_fixture, runtime, artifact) = service_fixture();
    let first = service_identity();
    let second = service_identity();
    runtime
        .prepare_service(
            first,
            std::slice::from_ref(&artifact),
            &serde_json::json!({}),
        )
        .expect("first service");
    runtime
        .prepare_service(
            second,
            std::slice::from_ref(&artifact),
            &serde_json::json!({}),
        )
        .expect("second service");

    let records = runtime.enumerate_service_instances().unwrap();
    assert_eq!(records.len(), 2);
    let mismatch = GatewayServiceIdentity {
        gateway_id: Uuid::new_v4(),
        ..first
    };
    assert!(
        runtime
            .prepare_service(
                mismatch,
                std::slice::from_ref(&artifact),
                &serde_json::json!({})
            )
            .is_err()
    );
    assert!(runtime.destroy_service(mismatch).is_err());
    assert!(runtime.service_path(first.instance_id).exists());

    runtime.destroy_service(first).expect("first cleanup");
    assert!(runtime.service_path(second.instance_id).exists());
    runtime.destroy_service(second).expect("second cleanup");
    assert!(runtime.enumerate_service_instances().unwrap().is_empty());
}

#[test]
fn service_symlinks_and_malicious_paths_fail_without_cross_namespace_deletion() {
    let (_fixture, runtime, artifact) = service_fixture();
    let identity = service_identity();
    let outside = runtime.runtime_root.join("outside");
    fs::create_dir(&outside).expect("outside");
    fs::write(outside.join("keep"), b"keep").expect("outside sentinel");
    let namespace = runtime.ensure_service_namespace().expect("namespace");
    symlink(&outside, namespace.join(identity.instance_id.to_string())).expect("instance symlink");
    assert!(runtime.destroy_service(identity).is_err());
    assert!(outside.join("keep").exists());
    assert!(runtime.enumerate_service_instances().is_err());

    fs::remove_file(namespace.join(identity.instance_id.to_string())).expect("symlink");
    let staging_id = Uuid::new_v4();
    let staging = namespace.join(format!("{GATEWAY_SERVICE_STAGING_PREFIX}{staging_id}"));
    symlink(&outside, &staging).expect("staging symlink");
    assert!(runtime.cleanup_service_staging().is_err());
    assert!(outside.join("keep").exists());
    fs::remove_file(&staging).expect("staging symlink");

    let broken = namespace.join(identity.instance_id.to_string());
    symlink("missing-target", &broken).expect("broken instance symlink");
    assert!(
        runtime
            .prepare_service(identity, &[artifact], &serde_json::json!({}))
            .is_err()
    );
    fs::remove_file(&broken).expect("broken instance symlink");

    fs::remove_dir(&namespace).expect("namespace");
    symlink(&outside, &namespace).expect("namespace symlink");
    assert!(runtime.enumerate_service_instances().is_err());
    assert!(runtime.cleanup_service_staging().is_err());
    assert!(outside.join("keep").exists());
}

#[test]
fn failed_service_materialization_cleans_staging_and_scoped_cleanup_is_safe() {
    let (_fixture, runtime, mut artifact) = service_fixture();
    artifact.content_hash = [0; 32];
    let identity = service_identity();
    assert!(
        runtime
            .prepare_service(
                identity,
                std::slice::from_ref(&artifact),
                &serde_json::json!({})
            )
            .is_err()
    );
    let namespace = runtime.service_namespace();
    assert_eq!(fs::read_dir(&namespace).unwrap().count(), 0);

    let staging_id = Uuid::new_v4();
    let staging = namespace.join(format!("{GATEWAY_SERVICE_STAGING_PREFIX}{staging_id}"));
    fs::create_dir(&staging).expect("staging");
    fs::write(staging.join("partial"), b"partial").expect("partial");
    let outside = runtime.runtime_root.join("outside");
    fs::create_dir(&outside).expect("outside");
    fs::write(outside.join("keep"), b"keep").expect("outside sentinel");
    assert_eq!(runtime.cleanup_service_staging().unwrap(), 1);
    assert!(!staging.exists());
    assert!(outside.join("keep").exists());
}
