//! Local negative controls are not native cgroup or absence qualification.
use super::{inspect_runtime, kernel};
use crate::provider::ownership::{LOCK, MARKER, ProviderOwner, SUPERVISOR};
use std::{fs, os::unix::fs::symlink};
use tempfile::TempDir;
use vm_trait::{VmError, VmProviderOwnerScope};

fn fixture() -> (TempDir, crate::LibkrunConfig, ProviderOwner) {
    let temp = TempDir::new().unwrap();
    let config = crate::provider::ownership::tests::config(&temp);
    let scope = VmProviderOwnerScope::new(uuid::Uuid::new_v4().to_string(), "observer-host".into())
        .unwrap();
    let owner = ProviderOwner::bootstrap(&config, &scope).unwrap();
    (temp, config, owner)
}

fn metadata(config: &crate::LibkrunConfig) -> Vec<Vec<u8>> {
    [MARKER, LOCK, SUPERVISOR]
        .map(|name| fs::read(config.runtime_root.join(name)).unwrap())
        .into()
}

#[test]
fn runtime_unknown_partial_birth_directory_and_socket_are_preserved() {
    let (_temp, config, owner) = fixture();
    let guard = owner.validate(&config).unwrap();
    let before = metadata(&config);
    for name in ["unknown", ".heph-vm-birth-partial.json"] {
        let path = config.runtime_root.join(name);
        fs::write(&path, b"partial original").unwrap();
        assert!(inspect_runtime(&owner, &config, &guard).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"partial original");
        fs::remove_file(path).unwrap(); // Test-owned input removal, never observer repair.
    }
    let path = config.runtime_root.join("residual-child");
    fs::create_dir(&path).unwrap();
    assert!(inspect_runtime(&owner, &config, &guard).is_err());
    assert!(path.is_dir());
    fs::remove_dir(path).unwrap();
    let path = config.runtime_root.join("worker.sock");
    let socket = std::os::unix::net::UnixListener::bind(&path).unwrap();
    assert!(inspect_runtime(&owner, &config, &guard).is_err());
    assert!(path.exists());
    drop(socket);
    assert_eq!(metadata(&config), before);
}

#[test]
fn runtime_symlink_and_same_bytes_replaced_metadata_are_denied() {
    for name in [MARKER, LOCK, SUPERVISOR] {
        let (_temp, config, owner) = fixture();
        let guard = owner.validate(&config).unwrap();
        let path = config.runtime_root.join(name);
        let held = path.with_extension("original");
        let bytes = fs::read(&path).unwrap();
        fs::rename(&path, &held).unwrap();
        fs::copy(&held, &path).unwrap();
        assert!(owner.validate(&config).is_err(), "replaced {name}");
        assert!(inspect_runtime(&owner, &config, &guard).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    let (_temp, config, owner) = fixture();
    let guard = owner.validate(&config).unwrap();
    let before = metadata(&config);
    let link = config.runtime_root.join("unknown-link");
    symlink(config.runtime_root.join(MARKER), &link).unwrap();
    assert!(inspect_runtime(&owner, &config, &guard).is_err());
    assert!(fs::symlink_metadata(link).unwrap().is_symlink());
    assert_eq!(metadata(&config), before);
}

#[test]
fn original_root_replacement_is_denied_without_touching_replacement() {
    let (temp, config, owner) = fixture();
    let original = temp.path().join("original-runtime");
    fs::rename(&config.runtime_root, &original).unwrap();
    fs::create_dir(&config.runtime_root).unwrap();
    assert!(owner.validate(&config).is_err());
    assert_eq!(fs::read_dir(&config.runtime_root).unwrap().count(), 0);
    assert!(original.join(MARKER).is_file());
}

#[test]
fn old_protocol_and_unenforced_or_fake_kernel_roots_never_prove_absence() {
    let (_temp, mut config, owner) = fixture();
    let guard = owner.validate(&config).unwrap();
    let before = metadata(&config);
    assert!(matches!(
        owner.assert_no_prior_births(&config, &guard),
        Err(VmError::Unsupported { .. })
    ));
    config.enforce_cgroup_v2 = true;
    fs::write(
        config.cgroup_root.join("cgroup.controllers"),
        b"cpu memory\n",
    )
    .unwrap();
    fs::write(
        config.cgroup_root.join("cgroup.events"),
        b"populated 0\nfrozen 0\n",
    )
    .unwrap();
    assert!(owner.assert_no_prior_births(&config, &guard).is_err());
    assert!(kernel::inspect(&guard.cgroup, &guard.cgroup).is_err());
    assert_eq!(metadata(&config), before);
    drop(guard);
    drop(owner);
    let path = config.runtime_root.join(MARKER);
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["version"] = 2.into();
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let owner = ProviderOwner::initialize(&config, "observer-host").unwrap();
    let guard = owner.validate(&config).unwrap();
    let before = metadata(&config);
    assert!(matches!(
        owner.assert_no_prior_births(&config, &guard),
        Err(VmError::Unsupported { .. })
    ));
    assert_eq!(metadata(&config), before);
}

#[test]
fn kernel_control_syntax_denies_missing_duplicate_unknown_and_invalid_values() {
    for text in [
        "",
        "populated 0\n",
        "populated 0\nfrozen 2\n",
        "populated 0\nfrozen 0\nfrozen 0\n",
        "populated 0\nfrozen 0\nunknown 0\n",
    ] {
        assert!(kernel::events(text).is_err(), "{text:?}");
    }
    for text in ["cpu cpu", "memory 1", "cpu\0", "CPU"] {
        assert!(kernel::controllers(text).is_err(), "{text:?}");
    }
}
