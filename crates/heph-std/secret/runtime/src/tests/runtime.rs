//! Secret runtime filesystem and lifecycle tests.

use super::{
    EphemeralSecretConfig, GUEST_SECRET_PATH, RUNTIME_CREDENTIAL_FILE, RawSecretFile,
    SecretMountState, SecretRuntimeError, materialize, materialize_with_authority,
    reconcile_orphans,
};
use runtime_types::RunId;
use secret_domain::{OpaqueRuntimeCredential, SecretSlotKey, SecretValue};
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};
use uuid::Uuid;

const SENTINEL: &[u8] = b"raw-mount-sentinel-71cf";

fn config(temporary: &tempfile::TempDir) -> EphemeralSecretConfig {
    let root = temporary.path().join("secret-mounts");
    fs::create_dir(&root).expect("create root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("secure root");
    EphemeralSecretConfig {
        root,
        require_memory_filesystem: false,
    }
}

fn raw(slot: &str, value: &[u8]) -> RawSecretFile {
    RawSecretFile {
        slot: SecretSlotKey::parse(slot).expect("valid slot"),
        value: SecretValue::new(value).expect("valid value"),
    }
}

#[test]
fn exact_read_only_contract_and_ordered_cleanup() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let config = config(&temporary);
    let credential = OpaqueRuntimeCredential::new([19_u8; 32]).expect("runtime credential");
    let mut mount = materialize_with_authority(
        &config,
        RunId::new(),
        vec![raw("model", SENTINEL)],
        &credential,
    )
    .expect("materialize");
    let file = mount.host_path().join("model");
    let authority = mount.host_path().join(RUNTIME_CREDENTIAL_FILE);
    assert_eq!(fs::read(&file).expect("read exact file"), SENTINEL);
    assert_eq!(
        fs::read(&authority).expect("read exact authority"),
        credential.expose()
    );
    assert_eq!(
        fs::metadata(&file)
            .expect("file metadata")
            .permissions()
            .mode()
            & 0o777,
        0o400
    );
    assert_eq!(
        fs::metadata(&authority)
            .expect("authority metadata")
            .permissions()
            .mode()
            & 0o777,
        0o400
    );
    assert_eq!(
        fs::metadata(mount.host_path())
            .expect("directory metadata")
            .permissions()
            .mode()
            & 0o777,
        0o500
    );
    let vm_mount = mount.vm_mount();
    assert_eq!(vm_mount.guest_path.to_string_lossy(), GUEST_SECRET_PATH);
    assert!(vm_mount.read_only);
    assert!(matches!(
        mount.destroy(),
        Err(SecretRuntimeError::GuestStillExists)
    ));
    mount.mark_guest_destroyed().expect("guest destroyed");
    mount.destroy().expect("clean secret mount");
    assert_eq!(mount.state(), SecretMountState::Destroyed);
    assert!(
        !temporary
            .path()
            .to_string_lossy()
            .contains(std::str::from_utf8(SENTINEL).expect("sentinel UTF-8"))
    );
}

#[test]
fn rejects_duplicates_bounds_and_unsafe_orphans() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let config = config(&temporary);
    let duplicated = materialize(
        &config,
        RunId::new(),
        vec![raw("model", b"a"), raw("model", b"b")],
    );
    assert!(matches!(duplicated, Err(SecretRuntimeError::DuplicateSlot)));

    let unsafe_path = config.root.join(Uuid::new_v4().simple().to_string());
    symlink(temporary.path(), &unsafe_path).expect("unsafe orphan symlink");
    assert!(matches!(
        reconcile_orphans(&config, &BTreeSet::new()),
        Err(SecretRuntimeError::UnsafeObject)
    ));
    fs::remove_file(unsafe_path).expect("remove test symlink");
}

#[test]
fn reconciles_only_opaque_inactive_directories() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let config = config(&temporary);
    let live =
        materialize(&config, RunId::new(), vec![raw("live", b"a")]).expect("live materialization");
    let orphan = materialize(&config, RunId::new(), vec![raw("orphan", b"b")])
        .expect("orphan materialization");
    let live_name = live
        .host_path()
        .file_name()
        .expect("live name")
        .to_string_lossy()
        .into_owned();
    let live_set = BTreeSet::from([live_name]);
    assert_eq!(reconcile_orphans(&config, &live_set).expect("reconcile"), 1);
    assert!(live.host_path().exists());
    assert!(!orphan.host_path().exists());
}
