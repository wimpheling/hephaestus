use super::*;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::Path,
};
use tempfile::TempDir;

fn setup() -> (TempDir, LibkrunConfig, VmProviderOwnerScope) {
    let temp = TempDir::new().unwrap();
    let config = super::tests::config(&temp);
    let scope =
        VmProviderOwnerScope::new(uuid::Uuid::new_v4().to_string(), "configured-host".into())
            .unwrap();
    (temp, config, scope)
}

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    path: String,
    device: u64,
    inode: u64,
    uid: u32,
    mode: u32,
    links: u64,
    bytes: Vec<u8>,
}

fn snapshot(root: &Path) -> Vec<Entry> {
    fn visit(path: &Path, entries: &mut Vec<Entry>) {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!("snapshot: {error}"),
        };
        let bytes = if metadata.is_symlink() {
            fs::read_link(path)
                .unwrap()
                .as_os_str()
                .as_encoded_bytes()
                .to_vec()
        } else if metadata.is_file() {
            fs::read(path).unwrap()
        } else {
            Vec::new()
        };
        entries.push(Entry {
            path: path.display().to_string(),
            device: metadata.dev(),
            inode: metadata.ino(),
            uid: metadata.uid(),
            mode: metadata.mode(),
            links: metadata.nlink(),
            bytes,
        });
        if metadata.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                visit(&entry.unwrap().path(), entries);
            }
        }
    }
    let mut entries = Vec::new();
    visit(root, &mut entries);
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries
}

fn denied_unchanged(temp: &TempDir, config: &LibkrunConfig, scope: &VmProviderOwnerScope) {
    let before = snapshot(temp.path());
    assert!(ProviderOwner::open_existing(config, scope).is_err());
    assert_eq!(snapshot(temp.path()), before);
}

#[test]
fn configured_namespace_bootstrap_and_readonly_reopen_retain_original_pins() {
    if !isolated_positive(
        "provider::ownership::configured_tests::configured_namespace_bootstrap_and_readonly_reopen_retain_original_pins",
    ) {
        return;
    }
    let (temp, config, scope) = setup();
    let owner = ProviderOwner::bootstrap(&config, &scope).unwrap();
    assert_eq!(owner.scope().unwrap(), scope);
    assert!(owner.pinned_roots.is_some());
    denied_unchanged(&temp, &config, &scope);
    drop(owner);
    fs::create_dir(config.runtime_root.join("previous-vm")).unwrap();
    let before = snapshot(temp.path());
    let reopened = ProviderOwner::open_existing(&config, &scope).unwrap();
    assert_eq!(reopened.scope().unwrap(), scope);
    assert!(reopened.pinned_roots.is_some());
    reopened.validate(&config).unwrap();
    assert_eq!(snapshot(temp.path()), before);
    assert!(ProviderOwner::bootstrap(&config, &scope).is_err());
    drop(reopened);
}

#[test]
fn existing_open_never_creates_missing_roots_or_markers() {
    let (temp, mut config, scope) = setup();
    denied_unchanged(&temp, &config, &scope);
    fs::write(config.runtime_root.join("foreign"), b"unchanged").unwrap();
    denied_unchanged(&temp, &config, &scope);
    config.runtime_root = temp.path().join("missing");
    denied_unchanged(&temp, &config, &scope);
}

#[test]
fn bootstrap_rejects_noncanonical_namespace_and_unknown_roots_before_writes() {
    let (temp, config, _) = setup();
    for namespace in [
        "configured-label",
        "00000000-0000-0000-0000-000000000000",
        "AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA",
    ] {
        let scope = VmProviderOwnerScope::new(namespace.into(), "configured-host".into()).unwrap();
        let before = snapshot(temp.path());
        assert!(ProviderOwner::bootstrap(&config, &scope).is_err());
        assert_eq!(snapshot(temp.path()), before);
    }
    let scope =
        VmProviderOwnerScope::new(uuid::Uuid::new_v4().to_string(), "configured-host".into())
            .unwrap();
    fs::create_dir(config.cgroup_root.join("old-vm")).unwrap();
    let before = snapshot(temp.path());
    assert!(ProviderOwner::bootstrap(&config, &scope).is_err());
    assert_eq!(snapshot(temp.path()), before);
    fs::remove_dir(config.cgroup_root.join("old-vm")).unwrap();
    fs::write(config.runtime_root.join(LOCK), b"incomplete bootstrap").unwrap();
    let before = snapshot(temp.path());
    assert!(ProviderOwner::bootstrap(&config, &scope).is_err());
    assert_eq!(snapshot(temp.path()), before);
}

#[test]
fn wrong_scope_host_cgroup_and_copied_root_are_untouched() {
    let (temp, config, scope) = setup();
    drop(ProviderOwner::bootstrap(&config, &scope).unwrap());
    for wrong in [
        VmProviderOwnerScope::new(uuid::Uuid::new_v4().to_string(), scope.host_id().into())
            .unwrap(),
        VmProviderOwnerScope::new(scope.namespace().into(), "other-host".into()).unwrap(),
    ] {
        denied_unchanged(&temp, &config, &wrong);
    }
    let mut redirected = config.clone();
    redirected.cgroup_root = temp.path().join("other-cgroup");
    fs::create_dir(&redirected.cgroup_root).unwrap();
    denied_unchanged(&temp, &redirected, &scope);
    redirected = config.clone();
    redirected.runtime_root = temp.path().join("copied-root");
    fs::create_dir(&redirected.runtime_root).unwrap();
    for name in [MARKER, LOCK, SUPERVISOR] {
        fs::copy(
            config.runtime_root.join(name),
            redirected.runtime_root.join(name),
        )
        .unwrap();
    }
    denied_unchanged(&temp, &redirected, &scope);
}

#[test]
fn missing_or_redirected_locks_and_partial_marker_are_not_repaired() {
    for name in [MARKER, LOCK, SUPERVISOR] {
        let (temp, config, scope) = setup();
        drop(ProviderOwner::bootstrap(&config, &scope).unwrap());
        fs::remove_file(config.runtime_root.join(name)).unwrap();
        denied_unchanged(&temp, &config, &scope);
        let foreign = temp.path().join("foreign");
        fs::write(&foreign, b"untouched").unwrap();
        symlink(&foreign, config.runtime_root.join(name)).unwrap();
        denied_unchanged(&temp, &config, &scope);
    }
    let (temp, config, scope) = setup();
    drop(ProviderOwner::bootstrap(&config, &scope).unwrap());
    fs::write(config.runtime_root.join(MARKER), b"{incomplete").unwrap();
    denied_unchanged(&temp, &config, &scope);
}

#[test]
fn stale_marker_and_replaced_supervisor_are_immutable_denials() {
    let (temp, config, scope) = setup();
    drop(ProviderOwner::bootstrap(&config, &scope).unwrap());
    let bytes = fs::read(config.runtime_root.join(MARKER)).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["version"] = 1.into();
    fs::write(
        config.runtime_root.join(MARKER),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    denied_unchanged(&temp, &config, &scope);
    fs::write(config.runtime_root.join(MARKER), bytes).unwrap();
    fs::rename(
        config.runtime_root.join(SUPERVISOR),
        temp.path().join("original-supervisor"),
    )
    .unwrap();
    fs::write(config.runtime_root.join(SUPERVISOR), b"").unwrap();
    fs::set_permissions(
        config.runtime_root.join(SUPERVISOR),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    denied_unchanged(&temp, &config, &scope);
}

#[test]
fn readonly_open_denies_unsafe_metadata_and_replaced_roots_without_mutation() {
    for name in [MARKER, LOCK, SUPERVISOR] {
        let (temp, config, scope) = setup();
        drop(ProviderOwner::bootstrap(&config, &scope).unwrap());
        let path = config.runtime_root.join(name);
        fs::hard_link(&path, temp.path().join("extra-link")).unwrap();
        denied_unchanged(&temp, &config, &scope);
        fs::remove_file(temp.path().join("extra-link")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
        denied_unchanged(&temp, &config, &scope);
    }
    let (temp, mut config, scope) = setup();
    let owner = ProviderOwner::bootstrap(&config, &scope).unwrap();
    fs::rename(&config.runtime_root, temp.path().join("original-runtime")).unwrap();
    fs::create_dir(&config.runtime_root).unwrap();
    fs::write(config.runtime_root.join("foreign"), b"untouched").unwrap();
    denied_unchanged(&temp, &config, &scope);
    let before = snapshot(temp.path());
    assert!(owner.validate(&config).is_err());
    assert_eq!(snapshot(temp.path()), before);
    drop(owner);
    config.runtime_root = temp.path().join("original-runtime");
    fs::rename(&config.cgroup_root, temp.path().join("original-cgroup")).unwrap();
    fs::create_dir(&config.cgroup_root).unwrap();
    denied_unchanged(&temp, &config, &scope);
}

pub(super) fn isolated_positive(name: &str) -> bool {
    if std::env::var("HEPH_CONFIGURED_OWNER_TEST_CHILD").as_deref() == Ok(name) {
        return true;
    }
    // Other parallel tests fork real children. A fresh child owns these new
    // descriptors independently, so unrelated parent forks cannot inherit them
    // until exec. Exact identity and live-supervisor negatives remain direct.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("HEPH_CONFIGURED_OWNER_TEST_CHILD", name)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "isolated constructor test failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}
