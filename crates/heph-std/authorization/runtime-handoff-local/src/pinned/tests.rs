//! Real descriptor/namespace controls, not VM or `PostgreSQL` custody evidence.

use super::*;
use std::os::unix::fs::symlink;

fn name() -> Name {
    Name::runtime(
        RuntimeSessionId::new(),
        RuntimeCredentialGeneration::INITIAL,
    )
}
fn private(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn held_original_leaf_refuses_same_bytes_replacement_and_preserves_both_inodes() {
    let directory = tempfile::tempdir().unwrap();
    let root = Root::initialize(directory.path()).unwrap();
    let name = name();
    let path = directory.path().join(&name.0);
    private(&path, b"original encrypted evidence");
    let leaf = root.acquire(directory.path(), name).unwrap().unwrap();
    let inode = leaf.file.metadata().unwrap().ino();
    let retained = directory.path().join("retained-original");
    fs::rename(&path, &retained).unwrap();
    private(&path, b"original encrypted evidence");
    assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
    assert!(root.unlink(directory.path(), &leaf).is_err());
    assert_eq!(leaf.file.metadata().unwrap().ino(), inode);
    assert_eq!(fs::read(&retained).unwrap(), b"original encrypted evidence");
    assert_eq!(fs::read(&path).unwrap(), b"original encrypted evidence");
}

#[test]
fn symlink_directory_hardlink_public_mode_and_oversized_leaf_all_deny_preserving_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let root = Root::initialize(directory.path()).unwrap();
    let external = directory.path().join("external");
    private(&external, b"unchanged");
    let symlink_name = name();
    let symlink_path = directory.path().join(&symlink_name.0);
    symlink(&external, &symlink_path).unwrap();
    assert!(root.acquire(directory.path(), symlink_name).is_err());
    assert!(
        fs::symlink_metadata(&symlink_path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let directory_name = name();
    let child = directory.path().join(&directory_name.0);
    fs::create_dir(&child).unwrap();
    assert!(root.acquire(directory.path(), directory_name).is_err());
    assert!(child.is_dir());
    let hardlink_name = name();
    let hardlink = directory.path().join(&hardlink_name.0);
    fs::hard_link(&external, &hardlink).unwrap();
    assert!(root.acquire(directory.path(), hardlink_name).is_err());
    assert!(hardlink.exists());
    let public_name = name();
    let public = directory.path().join(&public_name.0);
    private(&public, b"public mode denied");
    fs::set_permissions(&public, Permissions::from_mode(0o644)).unwrap();
    assert!(root.acquire(directory.path(), public_name).is_err());
    assert!(public.exists());
    let large_name = name();
    let large = directory.path().join(&large_name.0);
    private(
        &large,
        &vec![0; usize::try_from(MAX_ENVELOPE_BYTES + 1).unwrap()],
    );
    assert!(root.acquire(directory.path(), large_name).is_err());
    assert!(large.exists());
    assert_eq!(fs::read(external).unwrap(), b"unchanged");
}

#[test]
fn retained_root_refuses_replacement_mode_change_and_constructor_symlink_without_chmod_target() {
    let parent = tempfile::tempdir().unwrap();
    let path = parent.path().join("root");
    let root = Root::initialize(&path).unwrap();
    let name = name();
    let leaf_path = path.join(&name.0);
    private(&leaf_path, b"retained");
    let leaf = root.acquire(&path, name).unwrap().unwrap();
    let held = parent.path().join("held-root");
    fs::rename(&path, &held).unwrap();
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, Permissions::from_mode(0o700)).unwrap();
    assert!(root.validate(&path).is_err());
    assert!(root.unlink(&path, &leaf).is_err());
    assert_eq!(fs::read(held.join(&leaf.name.0)).unwrap(), b"retained");
    let mode_path = parent.path().join("mode-root");
    let mode_root = Root::initialize(&mode_path).unwrap();
    fs::set_permissions(&mode_path, Permissions::from_mode(0o750)).unwrap();
    assert!(mode_root.validate(&mode_path).is_err());
    let target = parent.path().join("target");
    fs::create_dir(&target).unwrap();
    fs::set_permissions(&target, Permissions::from_mode(0o750)).unwrap();
    let linked = parent.path().join("linked");
    symlink(&target, &linked).unwrap();
    assert!(Root::initialize(&linked).is_err());
    assert_eq!(fs::metadata(target).unwrap().mode() & 0o7777, 0o750);
}

#[test]
fn same_inode_in_place_edit_after_acquisition_denies_checked_unlink() {
    let directory = tempfile::tempdir().unwrap();
    let root = Root::initialize(directory.path()).unwrap();
    let name = name();
    let path = directory.path().join(&name.0);
    private(&path, b"original");
    let leaf = root.acquire(directory.path(), name).unwrap().unwrap();
    fs::write(&path, b"changed length").unwrap();
    assert!(root.unlink(directory.path(), &leaf).is_err());
    assert_eq!(fs::read(path).unwrap(), b"changed length");
}
