//! Ordinary filesystem substitution boundaries, not delegated-cgroup proof.
use super::ChildDirectory;
use std::{fs, os::unix::fs::symlink};

#[test]
fn observed_replacement_preserves_foreign_and_original_until_exact_restore() {
    let temp = tempfile::tempdir().unwrap();
    let child = ChildDirectory::create(
        temp.path(),
        "vm",
        rustix::fs::Mode::from_bits_truncate(0o700),
        |_| {},
    )
    .unwrap();
    let original = temp.path().join("original");
    fs::rename(temp.path().join("vm"), &original).unwrap();
    fs::create_dir(temp.path().join("vm")).unwrap();
    fs::write(temp.path().join("vm/sentinel"), b"foreign").unwrap();
    fs::write(child.path().unwrap().join("owned"), b"original").unwrap();
    assert_eq!(fs::read(original.join("owned")).unwrap(), b"original");
    assert!(child.remove(true).is_err());
    assert_eq!(
        fs::read(temp.path().join("vm/sentinel")).unwrap(),
        b"foreign"
    );
    assert_eq!(fs::read(original.join("owned")).unwrap(), b"original");
    fs::rename(temp.path().join("vm"), temp.path().join("foreign")).unwrap();
    fs::rename(original, temp.path().join("vm")).unwrap();
    child.remove(true).unwrap();
    child.remove(true).unwrap();
    assert!(!temp.path().join("vm").exists());
    assert_eq!(
        fs::read(temp.path().join("foreign/sentinel")).unwrap(),
        b"foreign"
    );
}

#[test]
fn exclusive_mkdir_with_unresolved_open_never_authorizes_named_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let foreign = temp.path().join("foreign");
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join("sentinel"), b"foreign").unwrap();
    let mut retained = None;
    let result = ChildDirectory::create(
        temp.path(),
        "vm",
        rustix::fs::Mode::from_bits_truncate(0o700),
        |child| {
            retained = Some(child);
            fs::rename(temp.path().join("vm"), temp.path().join("original")).unwrap();
            symlink(&foreign, temp.path().join("vm")).unwrap();
        },
    );
    assert!(result.is_err());
    let child = retained.unwrap();
    assert!(child.file().is_err());
    assert!(child.remove(true).is_err());
    assert!(temp.path().join("original").is_dir());
    assert_eq!(fs::read(foreign.join("sentinel")).unwrap(), b"foreign");
}

#[test]
fn collision_never_acquires_foreign_child_and_recursive_cleanup_does_not_follow_links() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("foreign")).unwrap();
    fs::write(temp.path().join("foreign/sentinel"), b"foreign").unwrap();
    let mut acquired = false;
    assert!(
        ChildDirectory::create(
            temp.path(),
            "foreign",
            rustix::fs::Mode::from_bits_truncate(0o700),
            |_| acquired = true
        )
        .is_err()
    );
    assert!(!acquired);
    let child = ChildDirectory::create(
        temp.path(),
        "vm",
        rustix::fs::Mode::from_bits_truncate(0o700),
        |_| {},
    )
    .unwrap();
    fs::create_dir(child.path().unwrap().join("nested")).unwrap();
    fs::write(child.path().unwrap().join("nested/owned"), b"original").unwrap();
    symlink(
        temp.path().join("foreign"),
        child.path().unwrap().join("redirect"),
    )
    .unwrap();
    child.remove(true).unwrap();
    assert!(!temp.path().join("vm").exists());
    assert_eq!(
        fs::read(temp.path().join("foreign/sentinel")).unwrap(),
        b"foreign"
    );
}
