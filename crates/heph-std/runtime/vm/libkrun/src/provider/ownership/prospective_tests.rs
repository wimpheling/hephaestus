use super::*;
use crate::provider::LibkrunProvider;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
};
use tempfile::TempDir;
use vm_trait::VmProvider;

fn setup() -> (TempDir, LibkrunConfig, VmProviderOwnerScope) {
    let temp = TempDir::new().unwrap();
    let mut config = super::tests::config(&temp);
    config.kvm_device = temp.path().join("emulated-kvm");
    config.passt_binary = "/bin/true".into();
    fs::write(&config.kvm_device, b"").unwrap();
    let scope =
        VmProviderOwnerScope::new(uuid::Uuid::new_v4().to_string(), "prospective-host".into())
            .unwrap();
    (temp, config, scope)
}

fn empty(root: &std::path::Path) {
    assert_eq!(fs::read_dir(root).unwrap().count(), 0);
}

#[test]
fn preparing_and_dropping_preserves_original_empty_directories() {
    let (_temp, config, scope) = setup();
    let before = fs::metadata(&config.runtime_root).unwrap();
    let cgroup_before = fs::metadata(&config.cgroup_root).unwrap();
    let prepared = LibkrunProvider::prepare_owned_bootstrap(config.clone(), &scope).unwrap();
    empty(&config.runtime_root);
    empty(&config.cgroup_root);
    drop(prepared);
    for (path, original) in [
        (&config.runtime_root, before),
        (&config.cgroup_root, cgroup_before),
    ] {
        let after = fs::metadata(path).unwrap();
        assert_eq!(
            (after.dev(), after.ino(), after.mode(), after.nlink()),
            (
                original.dev(),
                original.ino(),
                original.mode(),
                original.nlink()
            )
        );
        empty(path);
    }
}

#[test]
fn publication_binds_original_roots_and_scope_with_existing_supervision() {
    if !super::configured_tests::isolated_positive(
        "provider::ownership::prospective_tests::publication_binds_original_roots_and_scope_with_existing_supervision",
    ) {
        return;
    }
    let (_temp, config, scope) = setup();
    let prepared = LibkrunProvider::prepare_owned_bootstrap(config.clone(), &scope).unwrap();
    let provider = prepared.publish().unwrap();
    assert_eq!(provider.owner_scope().unwrap(), scope);
    let mut names: Vec<_> = fs::read_dir(&config.runtime_root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    let mut expected: Vec<_> = [LOCK, MARKER, SUPERVISOR]
        .map(std::ffi::OsString::from)
        .into();
    expected.sort();
    assert_eq!(names, expected);
    assert!(LibkrunProvider::open_owned(config.clone(), &scope).is_err());
    drop(provider);
    let reopened = LibkrunProvider::open_owned(config, &scope).unwrap();
    assert_eq!(reopened.owner_scope().unwrap(), scope);
    drop(reopened);
}

#[test]
// Keep the prepared owner pinned across the intervening mutation until consuming publish.
#[allow(clippy::significant_drop_tightening)]
fn publication_refuses_each_replaced_root_without_writing_either_directory() {
    for runtime in [true, false] {
        let (temp, config, scope) = setup();
        let prepared = LibkrunProvider::prepare_owned_bootstrap(config.clone(), &scope).unwrap();
        let path = if runtime {
            &config.runtime_root
        } else {
            &config.cgroup_root
        };
        let original = temp.path().join("retained-original");
        fs::rename(path, &original).unwrap();
        fs::create_dir(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        let held = prepared.publish().err().unwrap();
        empty(&original);
        empty(&config.runtime_root);
        empty(&config.cgroup_root);
        drop(held);
    }
}

#[test]
// Keep the prepared owner pinned across the intervening mutation until consuming publish.
#[allow(clippy::significant_drop_tightening)]
fn publication_refuses_new_unclassified_resources_without_deleting_them() {
    for runtime in [true, false] {
        let (_temp, config, scope) = setup();
        let prepared = LibkrunProvider::prepare_owned_bootstrap(config.clone(), &scope).unwrap();
        let path = if runtime {
            &config.runtime_root
        } else {
            &config.cgroup_root
        };
        let foreign = path.join("foreign-birth");
        fs::create_dir(&foreign).unwrap();
        fs::write(foreign.join("sentinel"), b"original foreign bytes").unwrap();
        let identity = fs::metadata(&foreign).unwrap();
        let held = prepared.publish().err().unwrap();
        assert_eq!(
            fs::read(foreign.join("sentinel")).unwrap(),
            b"original foreign bytes"
        );
        assert_eq!(fs::metadata(&foreign).unwrap().ino(), identity.ino());
        assert!(!config.runtime_root.join(LOCK).exists());
        assert!(!config.runtime_root.join(SUPERVISOR).exists());
        assert!(!config.runtime_root.join(MARKER).exists());
        drop(held);
    }
}

#[test]
fn full_configuration_rejection_precedes_owner_metadata_writes() {
    let (_temp, config, scope) = setup();
    let changes: [fn(&mut LibkrunConfig); 6] = [
        |value| value.worker_binary = "/missing-prospective-worker".into(),
        |value| value.passt_binary = "/missing-prospective-passt".into(),
        |value| value.kvm_device = "/missing-prospective-kvm".into(),
        |value| value.limits.cpu_period_micros = 0,
        |value| value.limits.memory_max_bytes = 0,
        |value| value.limits.pids_max = 0,
    ];
    for change in changes {
        let mut invalid = config.clone();
        change(&mut invalid);
        assert!(LibkrunProvider::validate_configuration(&invalid).is_err());
        assert!(LibkrunProvider::prepare_owned_bootstrap(invalid.clone(), &scope).is_err());
        assert!(LibkrunProvider::bootstrap_owned(invalid, &scope).is_err());
        empty(&config.runtime_root);
        empty(&config.cgroup_root);
    }
}

#[test]
fn preparation_requires_canonical_scope_and_pristine_dedicated_roots() {
    let (_temp, config, _) = setup();
    for namespace in [
        "configured-label",
        "00000000-0000-0000-0000-000000000000",
        "AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA",
    ] {
        let scope = VmProviderOwnerScope::new(namespace.into(), "prospective-host".into()).unwrap();
        assert!(LibkrunProvider::prepare_owned_bootstrap(config.clone(), &scope).is_err());
        empty(&config.runtime_root);
        empty(&config.cgroup_root);
    }
    let scope =
        VmProviderOwnerScope::new(uuid::Uuid::new_v4().to_string(), "prospective-host".into())
            .unwrap();
    fs::write(
        config.runtime_root.join(LOCK),
        b"prior incomplete reservation",
    )
    .unwrap();
    assert!(LibkrunProvider::prepare_owned_bootstrap(config.clone(), &scope).is_err());
    assert_eq!(
        fs::read(config.runtime_root.join(LOCK)).unwrap(),
        b"prior incomplete reservation"
    );
    assert!(!config.runtime_root.join(MARKER).exists());
    assert!(!config.runtime_root.join(SUPERVISOR).exists());
}

#[test]
// Keep the prepared owner pinned across the intervening mutation until consuming publish.
#[allow(clippy::significant_drop_tightening)]
fn late_failure_retains_original_locks_only_while_actual_error_is_held() {
    if !super::configured_tests::isolated_positive(
        "provider::ownership::prospective_tests::late_failure_retains_original_locks_only_while_actual_error_is_held",
    ) {
        return;
    }
    let (temp, config, scope) = setup();
    let prepared = LibkrunProvider::prepare_owned_bootstrap(config.clone(), &scope).unwrap();
    let original = temp.path().join("original-runtime");
    let held = prepared
        .publish_after_reservation(|| {
            fs::rename(&config.runtime_root, &original).unwrap();
            fs::create_dir(&config.runtime_root).unwrap();
            fs::set_permissions(&config.runtime_root, fs::Permissions::from_mode(0o700)).unwrap();
            fs::write(
                config.runtime_root.join("foreign"),
                b"replacement remains foreign",
            )
            .unwrap();
            Ok(())
        })
        .err()
        .unwrap();
    let VmError::Provider { code, source, .. } = &held else {
        panic!("late publication must retain custody: {held}");
    };
    assert_eq!(code, "bootstrap_publication_held");
    assert!(
        source
            .source()
            .unwrap()
            .to_string()
            .contains("prospective VM owner root identity changed")
    );
    assert!(!original.join(MARKER).exists());
    assert!(!config.runtime_root.join(LOCK).exists());
    let mut locks = Vec::new();
    for name in [LOCK, SUPERVISOR] {
        let file = fs::File::open(original.join(name)).unwrap();
        assert!(
            rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive).is_err()
        );
        locks.push(file);
    }
    assert_eq!(
        fs::read(config.runtime_root.join("foreign")).unwrap(),
        b"replacement remains foreign"
    );
    drop(held);
    for file in &locks {
        rustix::fs::flock(file, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
    }
    assert!(original.join(LOCK).exists());
    assert!(original.join(SUPERVISOR).exists());
    assert!(!original.join(MARKER).exists());
    assert_eq!(
        fs::read(config.runtime_root.join("foreign")).unwrap(),
        b"replacement remains foreign"
    );
    drop(locks);
}

#[test]
// Keep the prepared owner pinned across the intervening mutation until consuming publish.
#[allow(clippy::significant_drop_tightening)]
fn publication_rechecks_native_prerequisites_before_reservation() {
    let (temp, mut config, scope) = setup();
    config.worker_binary = temp.path().join("prospective-worker");
    fs::write(&config.worker_binary, b"fixture executable prerequisite").unwrap();
    fs::set_permissions(&config.worker_binary, fs::Permissions::from_mode(0o700)).unwrap();
    let prepared = LibkrunProvider::prepare_owned_bootstrap(config.clone(), &scope).unwrap();
    fs::remove_file(&config.worker_binary).unwrap();
    let error = prepared.publish().err().unwrap();
    assert!(matches!(error, VmError::Unavailable { resource, .. } if resource == "worker_binary"));
    empty(&config.runtime_root);
    empty(&config.cgroup_root);
}
