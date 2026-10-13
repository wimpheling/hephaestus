//! Genuine filesystem preparations; no pending/owner records are fabricated.

use super::{VolumeRootOwner, marker};
use rustix::fs::FlockOperation;
use std::{
    fs::{self, File},
    os::unix::fs::MetadataExt,
    path::Path,
};
use volume_trait::VolumeRootNamespaceId;

fn namespace() -> VolumeRootNamespaceId {
    VolumeRootNamespaceId::from_uuid(uuid::Uuid::from_u128(81)).unwrap()
}

fn identity(path: &Path) -> (u64, u64, u32) {
    let metadata = fs::symlink_metadata(path).unwrap();
    (metadata.dev(), metadata.ino(), metadata.mode())
}

fn empty(path: &Path) {
    assert_eq!(fs::read_dir(path).unwrap().count(), 0);
}

#[test]
fn preparation_and_drop_create_no_records() {
    let root = tempfile::tempdir().unwrap();
    let before = identity(root.path());
    let prepared =
        VolumeRootOwner::prepare_empty_with_namespace(root.path(), "host", namespace()).unwrap();
    assert_eq!(identity(root.path()), before);
    empty(root.path());
    drop(prepared);
    assert_eq!(identity(root.path()), before);
    empty(root.path());
}

#[test]
fn publication_uses_original_identity_and_releases_only_transient_lock() {
    let root = tempfile::tempdir().unwrap();
    let before = identity(root.path());
    let prepared =
        VolumeRootOwner::prepare_empty_with_namespace(root.path(), "host", namespace()).unwrap();
    let owner = prepared.publish().unwrap();
    assert_eq!(owner.namespace_id(), namespace());
    assert_eq!(owner.root(), root.path());
    assert_eq!(owner.host(), "host");
    assert_eq!(identity(root.path()), before);
    owner.validate().unwrap();
    assert!(root.path().join(marker::NAME).is_file());
    assert!(!root.path().join(marker::PENDING).exists());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    let independent = File::open(root.path()).unwrap();
    rustix::fs::flock(&independent, FlockOperation::NonBlockingLockExclusive).unwrap();
    owner.validate().unwrap();
}

#[test]
fn replaced_path_after_preparation_publishes_neither_directory() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("root");
    let old = parent.path().join("original");
    fs::create_dir(&root).unwrap();
    let before = identity(&root);
    let prepared =
        VolumeRootOwner::prepare_empty_with_namespace(&root, "host", namespace()).unwrap();
    fs::rename(&root, &old).unwrap();
    fs::create_dir(&root).unwrap();
    let replacement = identity(&root);
    assert!(prepared.publish().is_err());
    assert_eq!(identity(&old), before);
    assert_eq!(identity(&root), replacement);
    empty(&old);
    empty(&root);
}

#[test]
fn new_foreign_entry_after_preparation_is_preserved_without_reservation() {
    let root = tempfile::tempdir().unwrap();
    let prepared =
        VolumeRootOwner::prepare_empty_with_namespace(root.path(), "host", namespace()).unwrap();
    let raw = root.path().join("foreign.raw");
    fs::write(&raw, b"original foreign bytes").unwrap();
    let before = identity(&raw);
    assert!(prepared.publish().is_err());
    assert_eq!(identity(&raw), before);
    assert_eq!(fs::read(&raw).unwrap(), b"original foreign bytes");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    assert!(!root.path().join(marker::NAME).exists());
    assert!(!root.path().join(marker::PENDING).exists());
}

#[test]
fn held_preparation_excludes_cooperating_prepare_and_old_entrypoint() {
    let root = tempfile::tempdir().unwrap();
    let prepared =
        VolumeRootOwner::prepare_empty_with_namespace(root.path(), "host", namespace()).unwrap();
    assert!(
        VolumeRootOwner::prepare_empty_with_namespace(root.path(), "host", namespace()).is_err()
    );
    assert!(
        VolumeRootOwner::bootstrap_empty_with_namespace(root.path(), "host", namespace()).is_err()
    );
    empty(root.path());
    drop(prepared);
    let retry =
        VolumeRootOwner::prepare_empty_with_namespace(root.path(), "host", namespace()).unwrap();
    retry.publish().unwrap().validate().unwrap();
}

#[test]
fn observed_nonempty_and_invalid_configuration_fail_without_writes() {
    let root = tempfile::tempdir().unwrap();
    let raw = root.path().join("foreign.raw");
    fs::write(&raw, b"keep").unwrap();
    let before = identity(&raw);
    assert!(
        VolumeRootOwner::prepare_empty_with_namespace(root.path(), "host", namespace()).is_err()
    );
    assert_eq!(identity(&raw), before);
    assert_eq!(fs::read(&raw).unwrap(), b"keep");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    let empty_root = tempfile::tempdir().unwrap();
    for host in ["", "invalid\n"] {
        assert!(
            VolumeRootOwner::prepare_empty_with_namespace(empty_root.path(), host, namespace())
                .is_err()
        );
        empty(empty_root.path());
    }
    let missing = empty_root.path().join("absent");
    assert!(VolumeRootOwner::prepare_empty_with_namespace(&missing, "host", namespace()).is_err());
    assert!(!missing.exists());
    empty(empty_root.path());
}
