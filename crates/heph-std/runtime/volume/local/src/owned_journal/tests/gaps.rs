use super::super::{
    FormatState, JournalError, OwnedJournal, OwnedJournalLock, filesystem, namespace,
};
use super::support::{Fixture, restart_lock};
use std::{fs, io::Write};

#[test]
fn empty_recorded_stage_precedes_allocation_publication_and_format_intent() {
    let fixture = Fixture::new();
    let mut lock = OwnedJournalLock::acquire(fixture.purpose.clone()).expect("lock");
    let mut journal = OwnedJournal::open(&mut lock).expect("claim namespace");
    let backing = journal.create_staging().expect("record empty inode");
    assert_eq!(backing.file().metadata().expect("empty metadata").len(), 0);
    assert!(!fixture.canonical().exists());
    assert_eq!(
        journal.format_state().expect("positive pre-format proof"),
        FormatState::NeverStarted
    );
    assert!(
        journal.publish(&backing).is_err(),
        "allocation evidence is mandatory"
    );
    assert!(journal.begin_first_format(&backing).is_err());
    backing
        .file()
        .set_len(fixture.purpose.capacity())
        .expect("trusted allocator simulation");
    journal
        .mark_allocated(&backing)
        .expect("durable allocation evidence");
    journal.publish(&backing).expect("publish recorded inode");
    assert!(fixture.canonical().exists());
    assert_eq!(
        journal
            .recover_recorded()
            .expect("canonical exact inode")
            .identity(),
        backing.identity()
    );
    journal
        .begin_first_format(&backing)
        .expect("format intent durable before caller starts mkfs");
    assert_eq!(
        journal.format_state().expect("ambiguous initial format"),
        FormatState::MayHaveStarted
    );
    assert!(
        matches!(
            journal.begin_first_format(&backing),
            Err(JournalError::RecoveryRequired(_))
        ),
        "replay cannot authorize a second format"
    );
}

#[test]
fn unrecorded_empty_orphan_is_retained_and_a_new_inode_is_used() {
    let fixture = Fixture::new();
    let orphan;
    {
        let mut lock = OwnedJournalLock::acquire(fixture.purpose.clone()).expect("lock");
        let journal = OwnedJournal::open(&mut lock).expect("namespace");
        let (_, stage) =
            namespace::reserve_attempt(&journal.directory, "attempt", &journal.birth.hash)
                .expect("durable staging attempt");
        let name = format!("stage-{stage}.raw");
        let file = filesystem::create_file(&journal.directory, &name).expect("exclusive empty raw");
        file.sync_all().expect("sync inode");
        journal.directory.sync_all().expect("sync namespace");
        orphan = fixture.namespace().join(name);
        // Crash before the inode record: no descriptor escaped for allocation.
    }
    let mut lock = restart_lock(&fixture.purpose);
    let mut journal = OwnedJournal::open(&mut lock).expect("reopen complete birth namespace");
    let backing = journal.create_staging().expect("fresh bounded attempt");
    assert!(orphan.exists());
    assert_eq!(fs::metadata(&orphan).expect("retained orphan").len(), 0);
    assert_ne!(
        filesystem::FileIdentity::of(&fs::File::open(&orphan).expect("inspect only"))
            .expect("orphan inode"),
        backing.identity()
    );
}

#[test]
fn exact_inode_recovers_rename_before_publication_record_without_new_file() {
    let fixture = Fixture::new();
    let identity;
    {
        let mut lock = OwnedJournalLock::acquire(fixture.purpose.clone()).expect("lock");
        let mut journal = OwnedJournal::open(&mut lock).expect("namespace");
        let backing = journal.create_staging().expect("birth inode");
        identity = backing.identity();
        backing
            .file()
            .set_len(fixture.purpose.capacity())
            .expect("trusted allocation simulation");
        journal.mark_allocated(&backing).expect("allocation proof");
        filesystem::rename_new(
            &journal.directory,
            &backing.record.name(),
            journal.guard.root(),
            &fixture.purpose.canonical_name(),
        )
        .expect("canonical publication syscall");
        journal
            .directory
            .sync_all()
            .expect("source directory barrier");
        journal
            .guard
            .root()
            .sync_all()
            .expect("target directory barrier");
        // Crash before published marker and any database progress commit.
    }
    let mut lock = restart_lock(&fixture.purpose);
    let mut journal = OwnedJournal::open(&mut lock).expect("reopen birth");
    let recovered = journal
        .recover_recorded()
        .expect("observe exact original inode");
    assert_eq!(recovered.identity(), identity);
    journal
        .publish(&recovered)
        .expect("record recovered publication");
    assert_eq!(
        journal.format_state().expect("format never started"),
        FormatState::NeverStarted
    );
    assert!(journal.create_staging().is_err());
}

#[test]
fn pending_or_corrupted_format_record_never_proves_never_started() {
    let fixture = Fixture::new();
    let mut lock = OwnedJournalLock::acquire(fixture.purpose.clone()).expect("lock");
    let mut journal = OwnedJournal::open(&mut lock).expect("namespace");
    let backing = journal.create_staging().expect("inode");
    backing
        .file()
        .set_len(fixture.purpose.capacity())
        .expect("allocation simulation");
    journal.mark_allocated(&backing).expect("allocated");
    journal.publish(&backing).expect("published");
    let mut pending = filesystem::create_file(&journal.directory, "format-intent.pending")
        .expect("incomplete publication");
    pending.write_all(b"torn").expect("partial write");
    assert!(journal.format_state().is_err());
    assert!(journal.begin_first_format(&backing).is_err());
    assert_eq!(
        backing.file().metadata().expect("retained file").len(),
        fixture.purpose.capacity()
    );
}

#[test]
fn durable_format_intent_survives_restart_and_only_readonly_recovery_is_returned() {
    let fixture = Fixture::new();
    let original;
    {
        let mut lock = OwnedJournalLock::acquire(fixture.purpose.clone()).expect("lock");
        let mut journal = OwnedJournal::open(&mut lock).expect("namespace");
        let backing = journal.create_staging().expect("inode birth");
        original = backing.identity();
        backing
            .file()
            .set_len(fixture.purpose.capacity())
            .expect("trusted allocation simulation");
        journal
            .mark_allocated(&backing)
            .expect("allocation evidence");
        journal.publish(&backing).expect("canonical publication");
        journal
            .begin_first_format(&backing)
            .expect("format intent before any child");
        // Cancellation or process death now cannot authorize another format.
    }
    let mut lock = restart_lock(&fixture.purpose);
    let journal = OwnedJournal::open(&mut lock).expect("durable birth");
    let backing = journal
        .recover_recorded()
        .expect("exact inode readonly observation");
    assert_eq!(backing.identity(), original);
    assert_eq!(
        journal
            .format_state()
            .expect("recorded initial-format ambiguity"),
        FormatState::MayHaveStarted
    );
    assert!(
        backing.file().set_len(0).is_err(),
        "ambiguous recovery descriptor is readonly"
    );
    assert!(journal.begin_first_format(&backing).is_err());
}

#[test]
fn truncated_birth_on_disk_is_retained_and_never_reprepared() {
    let fixture = Fixture::new();
    let mut lock = OwnedJournalLock::acquire(fixture.purpose.clone()).expect("lock");
    {
        let journal = OwnedJournal::open(&mut lock).expect("complete birth");
        assert_eq!(
            journal.format_state().expect("initial proof"),
            FormatState::NeverStarted
        );
    }
    let path = fixture.namespace().join("birth");
    let original = fs::read(&path).expect("complete bytes");
    fs::write(&path, &original[..original.len() - 1]).expect("simulate lost tail");
    assert!(OwnedJournal::open(&mut lock).is_err());
    assert_eq!(
        fs::read(path).expect("retained damaged marker"),
        original[..original.len() - 1]
    );
    assert!(!fixture.canonical().exists());
}
