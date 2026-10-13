use super::super::{
    JournalError, OwnedJournal, OwnedJournalLock, codec::Kind, filesystem, records,
};
use super::support::Fixture;
use std::{
    fs,
    os::unix::{
        fs::{PermissionsExt, symlink},
        process::ExitStatusExt,
    },
    process::{Command, Stdio},
};

#[test]
fn preexisting_canonical_backing_is_retained_without_namespace_adoption() {
    let fixture = Fixture::new();
    fs::write(
        fixture.canonical(),
        b"independent matching metadata would not prove birth",
    )
    .expect("foreign backing");
    let before = fs::read(fixture.canonical()).expect("original bytes");
    let mut lock = OwnedJournalLock::acquire(fixture.purpose.clone()).expect("lock");
    assert!(matches!(
        OwnedJournal::open(&mut lock),
        Err(JournalError::Conflict(_))
    ));
    assert_eq!(
        fs::read(fixture.canonical()).expect("retained independent bytes"),
        before
    );
    assert!(!fixture.namespace().exists());
}

#[test]
fn existing_namespace_without_valid_birth_is_never_adopted() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.namespace()).expect("independent namespace");
    fs::set_permissions(fixture.namespace(), fs::Permissions::from_mode(0o700))
        .expect("private namespace");
    let mut lock = OwnedJournalLock::acquire(fixture.purpose.clone()).expect("lock");
    assert!(matches!(
        OwnedJournal::open(&mut lock),
        Err(JournalError::RecoveryRequired(_))
    ));
    assert!(fixture.namespace().exists());
    assert!(!fixture.namespace().join("birth").exists());
}

#[test]
fn symlink_birth_and_hardlinked_backing_are_rejected() {
    let fixture = Fixture::new();
    let mut lock = OwnedJournalLock::acquire(fixture.purpose.clone()).expect("lock");
    {
        let mut journal = OwnedJournal::open(&mut lock).expect("birth");
        let backing = journal.create_staging().expect("recorded inode");
        fs::hard_link(
            fixture.namespace().join(backing.record.name()),
            fixture.root.path().join("foreign-hardlink"),
        )
        .expect("adversarial alias");
        assert!(journal.recover_recorded().is_err());
        assert!(journal.mark_allocated(&backing).is_err());
    }
    let birth = fixture.namespace().join("birth");
    let retained = fixture.root.path().join("retained-birth");
    fs::rename(&birth, &retained).expect("retain actual marker");
    symlink(&retained, &birth).expect("adversarial marker symlink");
    assert!(OwnedJournal::open(&mut lock).is_err());
    assert!(retained.exists());
}

#[test]
fn replacement_inode_and_namespace_cannot_borrow_valid_birth_bytes() {
    let fixture = Fixture::new();
    let mut lock = OwnedJournalLock::acquire(fixture.purpose.clone()).expect("lock");
    {
        let mut journal = OwnedJournal::open(&mut lock).expect("birth");
        let backing = journal.create_staging().expect("original inode");
        let staged = fixture.namespace().join(backing.record.name());
        fs::rename(&staged, fixture.namespace().join("retained-original.raw"))
            .expect("hold original inode");
        fs::write(&staged, []).expect("same length different inode");
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o600))
            .expect("same private permissions");
        assert!(journal.recover_recorded().is_err());
        assert!(journal.publish(&backing).is_err());
    }
    let marker = fs::read(fixture.namespace().join("birth")).expect("valid original marker");
    fs::rename(
        fixture.namespace(),
        fixture.root.path().join("retained-original-namespace"),
    )
    .expect("hold original namespace inode");
    fs::create_dir(fixture.namespace()).expect("replacement namespace");
    fs::set_permissions(fixture.namespace(), fs::Permissions::from_mode(0o700))
        .expect("private replacement");
    fs::write(fixture.namespace().join("birth"), marker).expect("copy valid birth bytes");
    fs::set_permissions(
        fixture.namespace().join("birth"),
        fs::Permissions::from_mode(0o600),
    )
    .expect("private marker");
    assert!(
        OwnedJournal::open(&mut lock).is_err(),
        "namespace identity must match, even with valid full marker bytes"
    );
}

#[test]
fn valid_phase_checksum_cannot_skip_allocation_or_inherit_another_inode() {
    let fixture = Fixture::new();
    let mut lock = OwnedJournalLock::acquire(fixture.purpose).expect("lock");
    let mut journal = OwnedJournal::open(&mut lock).expect("birth");
    let backing = journal.create_staging().expect("inode");
    filesystem::publish_record(
        &journal.directory,
        "published",
        &records::phase(Kind::Published, &backing.record).expect("correct hash"),
    )
    .expect("adversarial skipped predecessor");
    assert!(journal.format_state().is_err());
    assert!(journal.begin_first_format(&backing).is_err());
}

#[test]
fn formatter_child_retains_existing_volume_lock_after_parent_drop() {
    let fixture = Fixture::new();
    let lock = OwnedJournalLock::acquire(fixture.purpose.clone()).expect("lock");
    let mut child = Command::new("/bin/sleep")
        .arg("1")
        .stdin(Stdio::from(lock.child_lock().expect("inherited flock")))
        .spawn()
        .expect("real lock-holding child");
    drop(lock);
    assert!(matches!(
        OwnedJournalLock::acquire(fixture.purpose.clone()),
        Err(JournalError::Locked)
    ));
    let status = child.wait().expect("child exit");
    assert!(
        status.success(),
        "formatter witness process failed: {:?}",
        status.signal()
    );
    assert!(OwnedJournalLock::acquire(fixture.purpose).is_ok());
}

#[test]
fn late_canonical_conflict_and_stage_symlink_preserve_foreign_bytes() {
    let fixture = Fixture::new();
    let mut lock = OwnedJournalLock::acquire(fixture.purpose.clone()).expect("lock");
    let mut journal = OwnedJournal::open(&mut lock).expect("namespace");
    let backing = journal.create_staging().expect("recorded birth");
    backing
        .file()
        .set_len(fixture.purpose.capacity())
        .expect("trusted allocation simulation");
    journal.mark_allocated(&backing).expect("allocated");
    let foreign = b"independently published backing must never be overwritten";
    fs::write(fixture.canonical(), foreign).expect("late independent backing");
    assert!(journal.publish(&backing).is_err());
    assert!(journal.format_state().is_err());
    assert_eq!(
        fs::read(fixture.canonical()).expect("retained foreign bytes"),
        foreign
    );
    let stage = fixture.namespace().join(backing.record.name());
    fs::rename(&stage, fixture.namespace().join("retained-stage.raw"))
        .expect("retain original inode");
    symlink(fixture.canonical(), &stage).expect("substituted raw symlink");
    assert!(journal.recover_recorded().is_err());
    assert_eq!(
        fs::read(fixture.canonical()).expect("no symlink target writes"),
        foreign
    );
}
