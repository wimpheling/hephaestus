use super::{VolumeRootOwner, marker};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use tempfile::TempDir;

#[test]
fn prospective_owner_retains_legacy_files_and_reopens_same_uuid() {
    let root = TempDir::new().expect("root");
    fs::write(root.path().join("legacy.raw"), b"independent").expect("legacy");
    let owner = VolumeRootOwner::initialize(root.path(), "host").expect("owner");
    let reopened = VolumeRootOwner::initialize(root.path(), "host").expect("reopen");
    assert_eq!(owner.namespace_id(), reopened.namespace_id());
    assert_eq!(
        fs::read(root.path().join("legacy.raw")).expect("legacy"),
        b"independent"
    );
    assert!(VolumeRootOwner::initialize(root.path(), "other-host").is_err());
}
#[test]
fn copied_marker_cannot_bind_a_different_root() {
    let first = TempDir::new().expect("first");
    let second = TempDir::new().expect("second");
    VolumeRootOwner::initialize(first.path(), "host").expect("owner");
    fs::copy(
        first.path().join(marker::NAME),
        second.path().join(marker::NAME),
    )
    .expect("copy");
    assert!(VolumeRootOwner::initialize(second.path(), "host").is_err());
    assert_eq!(fs::read_dir(second.path()).expect("entries").count(), 1);
}
#[test]
fn missing_owner_beside_owned_evidence_never_creates_a_new_identity() {
    let root = TempDir::new().expect("root");
    fs::create_dir(root.path().join(".owned-retained-pending")).expect("evidence");
    assert!(VolumeRootOwner::initialize(root.path(), "host").is_err());
    assert!(!root.path().join(marker::NAME).exists());
    assert!(!root.path().join(marker::PENDING).exists());
}
#[test]
fn root_substitution_invalidates_guard_without_foreign_mutations() {
    let parent = TempDir::new().expect("parent");
    let root = parent.path().join("volumes");
    fs::create_dir(&root).expect("root");
    let owner = VolumeRootOwner::initialize(&root, "host").expect("owner");
    fs::rename(&root, parent.path().join("retained")).expect("replace");
    fs::create_dir(&root).expect("foreign root");
    assert!(owner.validate().is_err());
    assert_eq!(fs::read_dir(root).expect("foreign entries").count(), 0);
}
#[test]
fn symlink_roots_and_markers_are_rejected_without_touching_targets() {
    let root = TempDir::new().expect("root");
    let outside = TempDir::new().expect("outside");
    symlink(outside.path(), root.path().join("linked-root")).expect("root symlink");
    assert!(VolumeRootOwner::initialize(&root.path().join("linked-root"), "host").is_err());
    fs::write(outside.path().join("retained"), b"retained").expect("file");
    symlink(
        outside.path().join("retained"),
        root.path().join(marker::NAME),
    )
    .expect("marker symlink");
    assert!(VolumeRootOwner::initialize(root.path(), "host").is_err());
    assert_eq!(
        fs::read(outside.path().join("retained")).expect("retained"),
        b"retained"
    );
}
#[test]
fn hardlinked_or_nonprivate_markers_fail_closed() {
    let root = TempDir::new().expect("root");
    let owner = VolumeRootOwner::initialize(root.path(), "host").expect("owner");
    fs::hard_link(root.path().join(marker::NAME), root.path().join("alias")).expect("alias");
    assert!(owner.validate().is_err());
    fs::remove_file(root.path().join("alias")).expect("unlink alias");
    fs::set_permissions(
        root.path().join(marker::NAME),
        fs::Permissions::from_mode(0o644),
    )
    .expect("permissions");
    assert!(owner.validate().is_err());
}
#[test]
fn truncated_or_pending_marker_never_mints_another_uuid() {
    let root = TempDir::new().expect("root");
    let owner = VolumeRootOwner::initialize(root.path(), "host").expect("owner");
    let path = root.path().join(marker::NAME);
    let bytes = fs::read(&path).expect("bytes");
    fs::write(&path, &bytes[..bytes.len() - 1]).expect("truncate");
    assert!(owner.validate().is_err());
    assert!(VolumeRootOwner::initialize(root.path(), "host").is_err());
    let other = TempDir::new().expect("other");
    fs::write(other.path().join(marker::PENDING), b"torn").expect("pending");
    assert!(VolumeRootOwner::initialize(other.path(), "host").is_err());
    assert!(!other.path().join(marker::NAME).exists());
}
