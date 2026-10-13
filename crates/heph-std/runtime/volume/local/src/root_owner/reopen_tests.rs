use super::{VolumeRootOwner, marker};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
};
use tempfile::TempDir;
use volume_trait::VolumeRootNamespaceId;

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    path: PathBuf,
    device: u64,
    inode: u64,
    uid: u32,
    mode: u32,
    bytes: Vec<u8>,
}

fn tree(path: &Path) -> Vec<Entry> {
    let mut pending = vec![path.to_path_buf()];
    let mut result = Vec::new();
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path).unwrap();
        let bytes = if metadata.file_type().is_symlink() {
            fs::read_link(&path)
                .unwrap()
                .as_os_str()
                .as_encoded_bytes()
                .to_vec()
        } else if metadata.is_file() {
            fs::read(&path).unwrap()
        } else {
            pending.extend(
                fs::read_dir(&path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
            Vec::new()
        };
        result.push(Entry {
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
            uid: metadata.uid(),
            mode: metadata.mode(),
            bytes,
        });
    }
    result.sort_by(|left, right| left.path.cmp(&right.path));
    result
}

fn denied_unchanged(path: &Path, host: &str, namespace: VolumeRootNamespaceId) {
    let before = tree(path);
    assert!(VolumeRootOwner::open_existing(path, host, namespace).is_err());
    assert_eq!(tree(path), before);
}

#[test]
fn readonly_reopen_preserves_exact_owner_and_all_file_inodes_bytes() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("legacy.raw"), b"never adopted").unwrap();
    let owner = VolumeRootOwner::initialize(root.path(), "host").unwrap();
    let before = tree(root.path());
    let reopened =
        VolumeRootOwner::open_existing(root.path(), "host", owner.namespace_id()).unwrap();
    assert_eq!(reopened.namespace_id(), owner.namespace_id());
    assert_eq!(reopened.host(), owner.host());
    assert_eq!(reopened.root(), owner.root());
    owner.validate().unwrap();
    reopened.validate().unwrap();
    assert_eq!(tree(root.path()), before);
}

#[test]
fn missing_root_is_not_created() {
    let parent = TempDir::new().unwrap();
    let namespace = VolumeRootNamespaceId::from_uuid(uuid::Uuid::new_v4()).unwrap();
    let before = tree(parent.path());
    assert!(
        VolumeRootOwner::open_existing(&parent.path().join("absent"), "host", namespace).is_err()
    );
    assert_eq!(tree(parent.path()), before);
}

#[test]
fn empty_or_unsealed_root_never_receives_a_marker() {
    let root = TempDir::new().unwrap();
    let namespace = VolumeRootNamespaceId::from_uuid(uuid::Uuid::new_v4()).unwrap();
    denied_unchanged(root.path(), "host", namespace);
    fs::write(root.path().join("independent.raw"), b"unsealed").unwrap();
    denied_unchanged(root.path(), "host", namespace);
    assert!(!root.path().join(marker::NAME).exists());
}

#[test]
fn empty_replacement_at_same_path_cannot_reopen_old_identity() {
    let parent = TempDir::new().unwrap();
    let root = parent.path().join("volumes");
    fs::create_dir(&root).unwrap();
    let owner = VolumeRootOwner::initialize(&root, "host").unwrap();
    fs::rename(&root, parent.path().join("retained")).unwrap();
    fs::create_dir(&root).unwrap();
    let before = tree(parent.path());
    denied_unchanged(&root, "host", owner.namespace_id());
    assert_eq!(tree(parent.path()), before);
    assert!(owner.validate().is_err());
}

#[test]
fn wrong_host_namespace_or_invalid_host_does_not_write() {
    let root = TempDir::new().unwrap();
    let owner = VolumeRootOwner::initialize(root.path(), "host").unwrap();
    denied_unchanged(root.path(), "other-host", owner.namespace_id());
    denied_unchanged(root.path(), "", owner.namespace_id());
    denied_unchanged(root.path(), "host\n", owner.namespace_id());
    denied_unchanged(root.path(), &"x".repeat(129), owner.namespace_id());
    denied_unchanged(
        root.path(),
        "host",
        VolumeRootNamespaceId::from_uuid(uuid::Uuid::new_v4()).unwrap(),
    );
}

#[test]
fn copied_marker_cannot_reopen_foreign_root() {
    let first = TempDir::new().unwrap();
    let second = TempDir::new().unwrap();
    let owner = VolumeRootOwner::initialize(first.path(), "host").unwrap();
    fs::copy(
        first.path().join(marker::NAME),
        second.path().join(marker::NAME),
    )
    .unwrap();
    let before = tree(first.path());
    denied_unchanged(second.path(), "host", owner.namespace_id());
    assert_eq!(tree(first.path()), before);
}

#[test]
fn symlink_root_or_marker_does_not_touch_the_target() {
    let parent = TempDir::new().unwrap();
    let root = TempDir::new().unwrap();
    let owner = VolumeRootOwner::initialize(root.path(), "host").unwrap();
    let link = parent.path().join("linked");
    symlink(root.path(), &link).unwrap();
    let before = tree(root.path());
    denied_unchanged(&link, "host", owner.namespace_id());
    assert_eq!(tree(root.path()), before);
    let marker_path = root.path().join(marker::NAME);
    let retained = root.path().join("retained-marker");
    fs::rename(&marker_path, &retained).unwrap();
    symlink(&retained, &marker_path).unwrap();
    denied_unchanged(root.path(), "host", owner.namespace_id());
}

#[test]
fn unsafe_root_marker_permissions_and_hardlinks_are_not_repaired() {
    let root = TempDir::new().unwrap();
    let owner = VolumeRootOwner::initialize(root.path(), "host").unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o777)).unwrap();
    denied_unchanged(root.path(), "host", owner.namespace_id());
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let marker_path = root.path().join(marker::NAME);
    fs::set_permissions(&marker_path, fs::Permissions::from_mode(0o644)).unwrap();
    denied_unchanged(root.path(), "host", owner.namespace_id());
    fs::set_permissions(&marker_path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::hard_link(&marker_path, root.path().join("alias")).unwrap();
    denied_unchanged(root.path(), "host", owner.namespace_id());
}

#[test]
fn pending_or_truncated_marker_stays_held_without_repair() {
    let root = TempDir::new().unwrap();
    let owner = VolumeRootOwner::initialize(root.path(), "host").unwrap();
    fs::write(root.path().join(marker::PENDING), b"incomplete").unwrap();
    denied_unchanged(root.path(), "host", owner.namespace_id());
    fs::remove_file(root.path().join(marker::PENDING)).unwrap();
    let path = root.path().join(marker::NAME);
    let bytes = fs::read(&path).unwrap();
    fs::write(&path, &bytes[..bytes.len() - 1]).unwrap();
    denied_unchanged(root.path(), "host", owner.namespace_id());
}
