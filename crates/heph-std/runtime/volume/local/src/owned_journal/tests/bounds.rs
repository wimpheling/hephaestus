use super::super::{
    JournalError, MAX_ATTEMPTS, OwnedJournal, OwnedJournalLock, filesystem, namespace,
};
use super::support::Fixture;
use rustix::fs::Mode;
use std::fs;

#[test]
fn interrupted_namespace_attempts_are_bounded_and_never_overwritten() {
    let fixture = Fixture::new();
    let mut lock = OwnedJournalLock::acquire(fixture.purpose.clone()).expect("lock");
    let prefix = format!("{}-namespace-attempt", fixture.purpose.namespace_name());
    for slot in 0..MAX_ATTEMPTS {
        filesystem::create_file(lock.root(), &format!("{prefix}-{slot:02}"))
            .expect("simulate interrupted reservation marker");
    }
    assert!(matches!(
        OwnedJournal::open(&mut lock),
        Err(JournalError::RecoveryRequired(_))
    ));
    assert!(!fixture.namespace().exists());
    let attempts = fs::read_dir(fixture.root.path())
        .expect("bounded fixture entries")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .count();
    assert_eq!(attempts, usize::from(MAX_ATTEMPTS));
}

#[test]
fn unrecorded_staging_attempts_exhaust_without_adopting_or_deleting_orphans() {
    let fixture = Fixture::new();
    let mut lock = OwnedJournalLock::acquire(fixture.purpose.clone()).expect("lock");
    let mut journal = OwnedJournal::open(&mut lock).expect("namespace");
    for _ in 0..MAX_ATTEMPTS {
        let (_, token) =
            namespace::reserve_attempt(&journal.directory, "attempt", &journal.birth.hash)
                .expect("bounded reservation");
        let orphan = filesystem::create_file(&journal.directory, &format!("stage-{token}.raw"))
            .expect("unrecorded empty inode");
        orphan.sync_all().expect("inode barrier");
        journal.directory.sync_all().expect("namespace barrier");
    }
    assert!(matches!(
        journal.create_staging(),
        Err(JournalError::RecoveryRequired(_))
    ));
    assert!(!fixture.canonical().exists());
    let stages = fs::read_dir(fixture.namespace())
        .expect("retained bounded orphans")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("stage-"))
        .count();
    assert_eq!(stages, usize::from(MAX_ATTEMPTS));
}

#[test]
fn unpublished_incomplete_namespace_is_retained_while_fresh_namespace_is_prepared() {
    let fixture = Fixture::new();
    let mut lock = OwnedJournalLock::acquire(fixture.purpose.clone()).expect("lock");
    let prefix = format!("{}-namespace-attempt", fixture.purpose.namespace_name());
    let (_, token) = namespace::reserve_attempt(
        lock.root(),
        &prefix,
        &super::super::codec::digest(fixture.purpose.bytes()),
    )
    .expect("namespace attempt before crash");
    let pending = format!("{}-pending-{token}", fixture.purpose.namespace_name());
    rustix::fs::mkdirat(lock.root(), &pending, Mode::RUSR | Mode::WUSR | Mode::XUSR)
        .expect("incomplete unpublished namespace");
    let journal = OwnedJournal::open(&mut lock).expect("fresh complete namespace");
    assert!(fixture.root.path().join(pending).exists());
    assert!(fixture.namespace().join("birth").exists());
    assert!(journal.format_state().is_ok());
}
