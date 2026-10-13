//! Original identity and interrupted-publication denials; controlled filesystem only.
use super::*;

#[test]
fn original_child_replacement_is_denied_before_cgroup_controls() {
    let fixture = Fixture::new();
    let current = fixture.config.runtime_root.join(&fixture.id.0);
    let old = fixture.config.runtime_root.join("old-child");
    fs::rename(&current, &old).unwrap();
    fs::create_dir(&current).unwrap();
    fs::write(current.join("foreign"), b"untouched").unwrap();
    assert!(fixture.recover().is_err());
    assert!(fixture.config.cgroup_root.join(&fixture.id.0).exists());
    assert_eq!(fs::read(current.join("foreign")).unwrap(), b"untouched");
    assert!(fixture.record_path().exists());
}

#[test]
fn unknown_allocation_without_original_record_is_held() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.record_path()).unwrap();
    assert!(fixture.recover().is_err());
    assert!(fixture.config.runtime_root.join(&fixture.id.0).exists());
    assert!(fixture.config.cgroup_root.join(&fixture.id.0).exists());
}

#[test]
fn published_record_substitutions_and_partial_bytes_do_not_become_originals() {
    let fixture = Fixture::new();
    let path = fixture.record_path();
    let bytes = fs::read(&path).unwrap();
    let original: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for (field, value) in [
        ("version", serde_json::json!(2)),
        ("birth", serde_json::json!(uuid::Uuid::nil())),
        ("vm_id", serde_json::json!("foreign")),
        ("namespace", serde_json::json!(uuid::Uuid::new_v4())),
        ("host_id", serde_json::json!("foreign-host")),
        ("unknown", serde_json::json!(true)),
    ] {
        let mut changed = original.clone();
        changed[field] = value;
        fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(fixture.recover().is_err(), "substitution {field}");
    }
    fs::write(&path, b"{").unwrap();
    assert!(fixture.recover().is_err());
    assert!(fixture.config.runtime_root.join(&fixture.id.0).exists());
    fs::write(&path, &bytes).unwrap();
    drop(fixture.recover().unwrap());
}

#[test]
fn recorded_runtime_and_cgroup_inode_substitutions_hold_both_originals() {
    for kind in ["runtime", "cgroup", "runtime_root", "cgroup_root"] {
        let fixture = Fixture::new();
        let path = fixture.record_path();
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value[kind]["inode"] = serde_json::json!(0);
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(fixture.recover().is_err(), "substitution {kind}");
        assert!(fixture.config.runtime_root.join(&fixture.id.0).exists());
        assert!(fixture.config.cgroup_root.join(&fixture.id.0).exists());
    }
}

#[test]
fn known_creation_cleanup_removes_its_record_without_directory_gate_deadlock() {
    let fixture = Fixture::new();
    fixture.cgroup.cleanup().unwrap();
    fixture.runtime.remove(true).unwrap();
    fixture.record.remove_original().unwrap();
    assert!(!fixture.record_path().exists());
    drop(fixture.recover().unwrap());
}

#[test]
fn interrupted_known_record_write_is_never_loaded_but_exact_creation_cleanup_can_retry() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let id = VmId("partial".into());
    let runtime = ChildDirectory::create(
        &fixture.config.runtime_root,
        &id.0,
        Mode::from_bits_truncate(0o700),
        |_| {},
    )
    .unwrap();
    let cgroup = Cgroup::create_tracked(&fixture.config, &id.0, |_| {}).unwrap();
    let guard = fixture.owner.validate(&fixture.config).unwrap();
    let path = fixture.config.runtime_root.join(filename(&id));
    let mut original = None;
    let result = BirthRecord::publish(
        (&guard.runtime, &guard.cgroup),
        &fixture.scope,
        &id,
        (
            &*runtime.file().unwrap(),
            &*cgroup.allocated_directory().unwrap().file().unwrap(),
        ),
        |record| {
            fs::write(&path, b"{").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
            original = Some(record);
        },
    );
    assert!(
        result.is_err(),
        "production publication failed before any spawn"
    );
    assert!(BirthRecord::load((&guard.runtime, &guard.cgroup), &fixture.scope, &id).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(BirthRecord::load((&guard.runtime, &guard.cgroup), &fixture.scope, &id).is_err());
    cgroup.cleanup().unwrap();
    runtime.remove(true).unwrap();
    original.unwrap().remove_original().unwrap();
    assert!(!path.exists());
}

#[test]
fn version_two_owner_is_unsupported_and_never_rewritten_or_adopted() {
    let Fixture {
        _directory: directory,
        config,
        owner,
        scope,
        ..
    } = Fixture::new();
    let marker = config.runtime_root.join(".heph-vm-owner.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();
    value["version"] = serde_json::json!(2);
    fs::write(&marker, serde_json::to_vec(&value).unwrap()).unwrap();
    let before = fs::read(&marker).unwrap();
    drop(owner);
    assert!(matches!(
        ProviderOwner::open_existing(&config, &scope),
        Err(VmError::Unsupported { .. })
    ));
    assert_eq!(fs::read(&marker).unwrap(), before);
    let diagnostic = ProviderOwner::initialize(&config, scope.host_id()).unwrap();
    assert!(matches!(
        diagnostic.require_containment(),
        Err(VmError::Unsupported { .. })
    ));
    assert_eq!(fs::read(&marker).unwrap(), before);
    assert!(config.runtime_root.join("birth").exists());
    assert!(directory.path().exists());
}
