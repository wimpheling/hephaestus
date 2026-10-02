use super::*;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    sync::Arc,
};
use tempfile::TempDir;

fn config(temp: &TempDir) -> LibkrunConfig {
    let runtime = temp.path().join("runtime");
    let cgroup = temp.path().join("cgroup");
    fs::create_dir(&runtime).unwrap();
    fs::create_dir(&cgroup).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&cgroup, fs::Permissions::from_mode(0o700)).unwrap();
    let mut config = LibkrunConfig::new(
        runtime,
        vec![temp.path().into()],
        vec![temp.path().into()],
        vec![temp.path().into()],
        "/bin/true",
        cgroup,
    );
    config.enforce_cgroup_v2 = false;
    config
}

#[test]
fn fresh_owner_survives_restart_and_rejects_host_redirect() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let owner = ProviderOwner::initialize(&config, "host-1").unwrap();
    let scope = owner.scope().unwrap();
    fs::create_dir(config.runtime_root.join("vm-1")).unwrap();
    let restarted = ProviderOwner::initialize(&config, "host-1").unwrap();
    assert_eq!(restarted.scope().unwrap(), scope);
    assert!(restarted.validate(&config).is_ok());
    assert!(ProviderOwner::initialize(&config, "host-2").is_err());
}

#[test]
fn concurrent_initialization_retains_one_durable_namespace() {
    let temp = TempDir::new().unwrap();
    let config = Arc::new(config(&temp));
    let mut threads = Vec::new();
    // Spawn every contender before joining so the real flock is exercised.
    for _ in 0..4 {
        let config = config.clone();
        threads.push(std::thread::spawn(move || {
            ProviderOwner::initialize(&config, "host-1")
                .unwrap()
                .scope()
                .unwrap()
        }));
    }
    let scopes: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert!(scopes.iter().all(|scope| scope == &scopes[0]));
}

#[test]
fn copied_marker_and_replaced_runtime_or_cgroup_cannot_claim_old_owner() {
    let temp = TempDir::new().unwrap();
    let mut config = config(&temp);
    let owner = ProviderOwner::initialize(&config, "host-1").unwrap();
    let original = config.runtime_root.clone();
    let replacement = temp.path().join("replacement");
    fs::create_dir(&replacement).unwrap();
    fs::copy(original.join(MARKER), replacement.join(MARKER)).unwrap();
    config.runtime_root = replacement;
    assert!(ProviderOwner::initialize(&config, "host-1").is_err());
    assert!(owner.validate(&config).is_err());
    config.runtime_root = original;
    let cgroup = temp.path().join("other-cgroup");
    fs::create_dir(&cgroup).unwrap();
    config.cgroup_root = cgroup;
    assert!(ProviderOwner::initialize(&config, "host-1").is_err());
    assert!(owner.validate(&config).is_err());
}

#[test]
fn nonempty_unclassified_and_symlink_roots_or_markers_fail_closed() {
    let temp = TempDir::new().unwrap();
    let mut config = config(&temp);
    fs::write(config.runtime_root.join("old-vm"), "unknown").unwrap();
    assert!(ProviderOwner::initialize(&config, "host-1").is_err());
    assert!(!config.runtime_root.join(MARKER).exists());
    fs::remove_file(config.runtime_root.join("old-vm")).unwrap();
    let original = config.runtime_root.clone();
    let alias = temp.path().join("alias");
    symlink(&original, &alias).unwrap();
    config.runtime_root = alias;
    assert!(ProviderOwner::initialize(&config, "host-1").is_err());
    config.runtime_root = original;
    let outside = temp.path().join("outside");
    fs::write(&outside, "untouched").unwrap();
    symlink(&outside, config.runtime_root.join(MARKER)).unwrap();
    assert!(ProviderOwner::initialize(&config, "host-1").is_err());
    assert_eq!(fs::read_to_string(outside).unwrap(), "untouched");
}

#[test]
fn changed_root_after_guard_is_detected_before_absence() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let owner = ProviderOwner::initialize(&config, "host-1").unwrap();
    let guard = owner.validate(&config).unwrap();
    fs::rename(&config.runtime_root, temp.path().join("old-runtime")).unwrap();
    fs::create_dir(&config.runtime_root).unwrap();
    assert!(owner.validate_guard(&config, &guard).is_err());
}

#[test]
fn old_cgroups_partial_metadata_and_redirected_lock_are_not_adopted() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let old = config.cgroup_root.join("unclassified-vm");
    fs::create_dir(&old).unwrap();
    assert!(ProviderOwner::initialize(&config, "host-1").is_err());
    assert!(!config.runtime_root.join(MARKER).exists());
    fs::remove_dir(old).unwrap();
    fs::write(config.runtime_root.join(MARKER), b"{incomplete").unwrap();
    fs::set_permissions(
        config.runtime_root.join(MARKER),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert!(ProviderOwner::initialize(&config, "host-1").is_err());
    fs::remove_file(config.runtime_root.join(MARKER)).unwrap();
    fs::remove_file(config.runtime_root.join(LOCK)).unwrap();
    let outside = temp.path().join("outside-lock");
    fs::write(&outside, b"untouched").unwrap();
    symlink(&outside, config.runtime_root.join(LOCK)).unwrap();
    assert!(ProviderOwner::initialize(&config, "host-1").is_err());
    assert_eq!(fs::read(outside).unwrap(), b"untouched");
}

#[test]
fn shared_physical_guards_coexist_without_relaxing_owner_validation() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let owner = ProviderOwner::initialize(&config, "host-1").unwrap();
    let first = owner.validate(&config).unwrap();
    let second = owner.validate(&config).unwrap();
    owner.validate_guard(&config, &first).unwrap();
    owner.validate_guard(&config, &second).unwrap();
    let lock: File = rustix::fs::openat(
        &first.runtime,
        LOCK,
        rustix::fs::OFlags::RDWR | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .unwrap()
    .into();
    assert!(
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive).is_err(),
        "initialization cannot mutate owner metadata during physical IO"
    );
    drop(lock);
    fs::rename(&config.runtime_root, temp.path().join("old-root")).unwrap();
    fs::create_dir(&config.runtime_root).unwrap();
    assert!(owner.validate_guard(&config, &first).is_err());
    assert!(owner.validate_guard(&config, &second).is_err());
}
