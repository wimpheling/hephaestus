use super::support::*;
use crate::config::IoLimit;
use vm_trait::VmProviderOwnerScope;

fn owned() -> (
    TempDir,
    LibkrunProvider,
    LibkrunConfig,
    VmProviderOwnerScope,
) {
    let temp = TempDir::new().unwrap();
    let (mut config, _, _, _) = emulated_config(&temp);
    config.passt_binary = "/bin/true".into();
    let scope = VmProviderOwnerScope::new(
        uuid::Uuid::new_v4().to_string(),
        "managed-configuration-test".into(),
    )
    .unwrap();
    let provider = LibkrunProvider::bootstrap_owned(config.clone(), &scope).unwrap();
    (temp, provider, config, scope)
}

fn owner_files(config: &LibkrunConfig) -> Vec<(std::ffi::OsString, Vec<u8>)> {
    let mut files: Vec<_> = fs::read_dir(&config.runtime_root)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), fs::read(entry.path()).unwrap())
        })
        .collect();
    files.sort();
    files
}

#[test]
fn exact_retained_configuration_and_owner_survive_provider_clone_without_writes() {
    let (_temp, provider, config, scope) = owned();
    let before = owner_files(&config);
    provider
        .verify_managed_configuration(&config, &scope)
        .unwrap();
    let clone = provider.clone();
    drop(provider);
    clone.verify_managed_configuration(&config, &scope).unwrap();
    assert_eq!(clone.owner_scope().unwrap(), scope);
    drop(clone);
    assert_eq!(owner_files(&config), before);
}

#[test]
fn configuration_substitutions_fail_against_the_retained_concrete_provider() {
    let (_temp, provider, config, scope) = owned();
    let before = owner_files(&config);
    let substitutions: [fn(&mut LibkrunConfig); 12] = [
        |value| value.runtime_root.push("other-root"),
        |value| value.cgroup_root.push("other-cgroup"),
        |value| value.image_roots.push("/unexpected/image".into()),
        |value| value.disk_roots.push("/unexpected/disk".into()),
        |value| value.mount_roots.push("/unexpected/mount".into()),
        |value| value.broker_socket_path = Some("/unexpected/broker.sock".into()),
        |value| value.runtime_git_socket_path = Some("/unexpected/git.sock".into()),
        |value| value.worker_binary = "/unexpected/worker".into(),
        |value| value.libkrun_library = "unexpected-libkrun.so".into(),
        |value| value.service_gid += 1,
        |value| value.startup_timeout += Duration::from_secs(1),
        |value| {
            value.limits.io.push(IoLimit {
                major: 8,
                minor: 1,
                read_bps: Some(4096),
                write_bps: None,
                read_iops: None,
                write_iops: None,
            });
        },
    ];
    for substitute in substitutions {
        let mut expected = config.clone();
        substitute(&mut expected);
        assert!(matches!(
            provider.verify_managed_configuration(&expected, &scope),
            Err(VmError::InvalidState(
                "managed VM provider configuration differs"
            ))
        ));
    }
    let mut expected = config.clone();
    expected.limits.memory_max_bytes += 1;
    assert!(
        provider
            .verify_managed_configuration(&expected, &scope)
            .is_err()
    );
    provider
        .verify_managed_configuration(&config, &scope)
        .unwrap();
    assert_eq!(owner_files(&config), before);
}

#[test]
fn wrong_namespace_or_host_is_denied_without_owner_metadata_changes() {
    let (_temp, provider, config, scope) = owned();
    let before = owner_files(&config);
    for wrong in [
        VmProviderOwnerScope::new(uuid::Uuid::new_v4().to_string(), scope.host_id().into())
            .unwrap(),
        VmProviderOwnerScope::new(scope.namespace().into(), "other-host".into()).unwrap(),
    ] {
        assert!(matches!(
            provider.verify_managed_configuration(&config, &wrong),
            Err(VmError::InvalidState(
                "managed VM provider owner scope differs"
            ))
        ));
    }
    assert_eq!(owner_files(&config), before);
}

#[test]
fn matching_configuration_cannot_admit_an_unmanaged_provider() {
    let temp = TempDir::new().unwrap();
    let (mut config, _, _, _) = emulated_config(&temp);
    config.passt_binary = "/bin/true".into();
    let provider = LibkrunProvider::new(config.clone()).unwrap();
    let expected =
        VmProviderOwnerScope::new("unmanaged-namespace".into(), "test-host".into()).unwrap();
    assert!(matches!(
        provider.verify_managed_configuration(&config, &expected),
        Err(VmError::Unsupported { .. })
    ));
    drop(provider);
    assert_eq!(fs::read_dir(&config.runtime_root).unwrap().count(), 0);
}

#[test]
fn missing_owner_marker_is_denied_and_never_repaired() {
    let (_temp, provider, config, scope) = owned();
    fs::remove_file(config.runtime_root.join(".heph-vm-owner.json")).unwrap();
    let before = owner_files(&config);
    assert!(
        provider
            .verify_managed_configuration(&config, &scope)
            .is_err()
    );
    assert_eq!(owner_files(&config), before);
}

#[test]
fn same_configured_path_with_replaced_runtime_root_is_denied_without_adoption() {
    let (temp, provider, config, scope) = owned();
    fs::rename(&config.runtime_root, temp.path().join("original-runtime")).unwrap();
    fs::create_dir(&config.runtime_root).unwrap();
    assert!(
        provider
            .verify_managed_configuration(&config, &scope)
            .is_err()
    );
    assert_eq!(fs::read_dir(&config.runtime_root).unwrap().count(), 0);
}
