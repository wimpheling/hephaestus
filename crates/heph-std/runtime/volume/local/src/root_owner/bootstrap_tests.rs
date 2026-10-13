use super::{VolumeRootOwner, bootstrap, marker, only_entry};
use crate::owned_journal::filesystem::FileIdentity;
use rustix::fs::FlockOperation;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, File},
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::Path,
    sync::{Arc, Barrier},
};
use volume_trait::VolumeRootNamespaceId;

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    device: u64,
    inode: u64,
    mode: u32,
    links: u64,
    bytes: Vec<u8>,
}
fn entry(path: &Path) -> Entry {
    let metadata = fs::symlink_metadata(path).unwrap();
    Entry {
        device: metadata.dev(),
        inode: metadata.ino(),
        mode: metadata.mode(),
        links: metadata.nlink(),
        bytes: if metadata.is_file() {
            fs::read(path).unwrap()
        } else {
            Vec::new()
        },
    }
}
#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    root: Entry,
    entries: BTreeMap<OsString, Entry>,
}
fn snapshot(path: &Path) -> Snapshot {
    Snapshot {
        root: entry(path),
        entries: fs::read_dir(path)
            .unwrap()
            .map(|item| {
                let item = item.unwrap();
                (item.file_name(), entry(&item.path()))
            })
            .collect(),
    }
}
fn namespace(value: u128) -> VolumeRootNamespaceId {
    VolumeRootNamespaceId::from_uuid(uuid::Uuid::from_u128(value)).unwrap()
}
fn denied_unchanged(path: &Path, host: &str, expected: VolumeRootNamespaceId) {
    let before = snapshot(path);
    assert!(VolumeRootOwner::bootstrap_empty_with_namespace(path, host, expected).is_err());
    assert_eq!(snapshot(path), before);
}

#[test]
fn configured_uuid_reopens_readonly_and_does_not_retain_bootstrap_lock() {
    let root = tempfile::tempdir().unwrap();
    let expected = namespace(71);
    let owner =
        VolumeRootOwner::bootstrap_empty_with_namespace(root.path(), "host", expected).unwrap();
    assert_eq!(owner.namespace_id(), expected);
    owner.validate().unwrap();
    let bytes = fs::read(root.path().join(marker::NAME)).unwrap();
    let header = b"HEPHVROOT1".len();
    assert_eq!(&bytes[header..header + 16], expected.as_uuid().as_bytes());
    let before = snapshot(root.path());
    let reopened = VolumeRootOwner::open_existing(root.path(), "host", expected).unwrap();
    assert_eq!(reopened.namespace_id(), expected);
    assert_eq!(snapshot(root.path()), before);
    assert!(VolumeRootOwner::open_existing(root.path(), "foreign", expected).is_err());
    assert!(VolumeRootOwner::open_existing(root.path(), "host", namespace(72)).is_err());
    assert_eq!(snapshot(root.path()), before);
    denied_unchanged(root.path(), "host", expected);
    // Both returned owners remain alive; a separate open description can lock.
    let lock = File::open(root.path()).unwrap();
    rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive).unwrap();
    owner.validate().unwrap();
    reopened.validate().unwrap();
}

#[test]
fn nonempty_and_interrupted_roots_are_never_adopted_or_repaired() {
    for name in [
        "legacy.raw",
        ".owned-existing",
        marker::NAME,
        marker::PENDING,
    ] {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(name), b"foreign or interrupted bytes").unwrap();
        denied_unchanged(root.path(), "host", namespace(73));
    }
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("foreign-directory");
    fs::create_dir(&directory).unwrap();
    let retained_file = directory.join("keep");
    fs::write(&retained_file, b"keep exact bytes").unwrap();
    let before = entry(&retained_file);
    denied_unchanged(root.path(), "host", namespace(74));
    assert_eq!(entry(&retained_file), before);
}

#[test]
fn missing_noncanonical_unsafe_and_invalid_inputs_create_nothing() {
    let parent = tempfile::tempdir().unwrap();
    let before = snapshot(parent.path());
    assert!(
        VolumeRootOwner::bootstrap_empty_with_namespace(
            &parent.path().join("missing"),
            "host",
            namespace(75)
        )
        .is_err()
    );
    assert_eq!(snapshot(parent.path()), before);
    let root = parent.path().join("root");
    fs::create_dir(&root).unwrap();
    let linked = parent.path().join("linked");
    symlink(&root, &linked).unwrap();
    let before = snapshot(parent.path());
    assert!(
        VolumeRootOwner::bootstrap_empty_with_namespace(&linked, "host", namespace(75)).is_err()
    );
    assert_eq!(snapshot(parent.path()), before);
    let before = snapshot(&root);
    assert!(
        VolumeRootOwner::bootstrap_empty_with_namespace(&root.join(".."), "host", namespace(75))
            .is_err()
    );
    assert_eq!(snapshot(&root), before);
    for host in [String::new(), "host\n".to_owned(), "x".repeat(129)] {
        denied_unchanged(&root, &host, namespace(75));
    }
    fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();
    denied_unchanged(&root, "host", namespace(75));
    assert!(VolumeRootNamespaceId::from_uuid(uuid::Uuid::nil()).is_err());
}

#[test]
fn cooperating_bootstraps_have_one_winner_and_contenders_do_not_write() {
    let root = tempfile::tempdir().unwrap();
    let lock = File::open(root.path()).unwrap();
    rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive).unwrap();
    denied_unchanged(root.path(), "host", namespace(76));
    drop(lock);
    let barrier = Arc::new(Barrier::new(2));
    let workers = [namespace(76), namespace(77)].map(|expected| {
        let path = root.path().to_owned();
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            VolumeRootOwner::bootstrap_empty_with_namespace(&path, "host", expected)
                .map(|owner| owner.namespace_id())
        })
    });
    let results = workers.map(|worker| worker.join().unwrap());
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let winner = results.into_iter().find_map(Result::ok).unwrap();
    let owner = VolumeRootOwner::open_existing(root.path(), "host", winner).unwrap();
    owner.validate().unwrap();
    assert!(!root.path().join(marker::PENDING).exists());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn replacement_before_publication_never_writes_the_foreign_root() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("root");
    let original = parent.path().join("original");
    fs::create_dir(&root).unwrap();
    let before = snapshot(&root);
    let mut foreign = None;
    let result = bootstrap(&root, "host", namespace(78), || {
        fs::rename(&root, &original).unwrap();
        fs::create_dir(&root).unwrap();
        fs::write(root.join("foreign.raw"), b"leave foreign data alone").unwrap();
        foreign = Some(snapshot(&root));
    });
    assert!(result.is_err());
    assert_eq!(snapshot(&original), before);
    assert_eq!(snapshot(&root), foreign.unwrap());
}

#[test]
fn racing_entry_after_reservation_keeps_interrupted_evidence_and_foreign_bytes() {
    let root = tempfile::tempdir().unwrap();
    let pinned = marker::open_root(root.path()).unwrap();
    let identity = FileIdentity::of(&pinned).unwrap();
    let raw = root.path().join("foreign.raw");
    let mut foreign = None;
    let result = marker::publish(&pinned, "host", identity, namespace(79), || {
        fs::write(&raw, b"racing foreign contents").unwrap();
        foreign = Some(entry(&raw));
        only_entry(&pinned, Some(marker::PENDING))
    });
    assert!(result.is_err());
    assert_eq!(entry(&raw), foreign.unwrap());
    assert_eq!(fs::read(root.path().join(marker::PENDING)).unwrap(), b"");
    assert!(!root.path().join(marker::NAME).exists());
    denied_unchanged(root.path(), "host", namespace(79));
    let before = snapshot(root.path());
    assert!(VolumeRootOwner::open_existing(root.path(), "host", namespace(79)).is_err());
    assert_eq!(snapshot(root.path()), before);
}

#[test]
fn legacy_initializer_keeps_unsealed_files_and_its_original_reopen_semantics() {
    let root = tempfile::tempdir().unwrap();
    let raw = root.path().join("unsealed.raw");
    fs::write(&raw, b"never adopt or rewrite").unwrap();
    let before = entry(&raw);
    let owner = VolumeRootOwner::initialize(root.path(), "host").unwrap();
    let expected = owner.namespace_id();
    assert_eq!(entry(&raw), before);
    assert_eq!(
        VolumeRootOwner::initialize(root.path(), "host")
            .unwrap()
            .namespace_id(),
        expected
    );
    VolumeRootOwner::open_existing(root.path(), "host", expected).unwrap();
    denied_unchanged(root.path(), "host", expected);
    assert_eq!(entry(&raw), before);
}
