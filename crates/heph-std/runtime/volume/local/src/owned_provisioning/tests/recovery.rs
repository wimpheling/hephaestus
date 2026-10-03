use super::support::Fixture;
use crate::owned_journal::{OwnedJournal, OwnedJournalLock};
use std::{fs, io::Write};

#[tokio::test]
async fn readonly_worker_finishes_real_birth_after_final_commit_gap_and_actor_loss() {
    let fixture = Fixture::new();
    fixture.state.lock().expect("state").fail_ready_once = true;
    assert!(
        fixture
            .store
            .provision_owned(&fixture.identity, &fixture.request)
            .await
            .is_err()
    );
    let bytes = fs::read(fixture.canonical()).expect("real formatted bytes");
    fixture.state.lock().expect("state").authorized = false;
    let before = fixture.state.lock().expect("state").begins;
    let context = fixture
        .store
        .reconcile_owned(&fixture.claim())
        .await
        .expect("readonly worker recovery");
    assert_eq!(
        context.observation.expect("proof").fields().phase,
        volume_trait::OwnedBackingPhase::Ready
    );
    assert_eq!(
        fixture.state.lock().expect("state").begins,
        before,
        "no historical actor authentication synthesized"
    );
    assert_eq!(fixture.format_count(), 1);
    assert_eq!(
        fs::read(fixture.canonical()).expect("retained bytes"),
        bytes
    );
}
#[tokio::test]
async fn exact_claimed_set_len_gap_finishes_allocation_without_resizing_and_formats_once() {
    let fixture = Fixture::new();
    let mut lock = OwnedJournalLock::acquire(fixture.journal_purpose()).expect("lock");
    let mut journal = OwnedJournal::open(&mut lock).expect("journal");
    let backing = journal.create_staging().expect("birth");
    journal.begin_allocation(&backing).expect("intent");
    backing
        .file()
        .set_len(16 * 1024 * 1024)
        .expect("allocated before crash");
    let original_inode = backing.identity();
    drop(backing);
    drop(journal);
    drop(lock);
    let context = fixture
        .store
        .provision_owned(&fixture.identity, &fixture.request)
        .await
        .expect("positive actor recovery");
    assert_eq!(
        context.observation.expect("proof").fields().backing_inode,
        original_inode.inode
    );
    assert_eq!(fixture.format_count(), 1);
}
#[tokio::test]
async fn creator_loss_denies_allocation_gap_resume_and_worker_cannot_finish_first_format() {
    let fixture = Fixture::new();
    let mut lock = OwnedJournalLock::acquire(fixture.journal_purpose()).expect("lock");
    let mut journal = OwnedJournal::open(&mut lock).expect("journal");
    let backing = journal.create_staging().expect("birth");
    backing
        .file()
        .set_len(16 * 1024 * 1024)
        .expect("allocation gap");
    drop(backing);
    drop(journal);
    drop(lock);
    fixture.state.lock().expect("state").authorized = false;
    assert!(
        fixture
            .store
            .provision_owned(&fixture.identity, &fixture.request)
            .await
            .is_err()
    );
    assert!(
        fixture
            .store
            .reconcile_owned(&fixture.claim())
            .await
            .is_err()
    );
    assert_eq!(fixture.format_count(), 0);
    assert!(!fixture.canonical().exists());
}
#[tokio::test]
async fn unrecorded_unpublished_orphan_is_retained_and_never_adopted() {
    let fixture = Fixture::new();
    let mut lock = OwnedJournalLock::acquire(fixture.journal_purpose()).expect("lock");
    let journal = OwnedJournal::open(&mut lock).expect("journal");
    let orphan = fixture
        .root
        .path()
        .join(fixture.request.purpose.namespace_name())
        .join("stage-unrecorded.raw");
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&orphan)
        .expect("orphan");
    file.write_all(b"unrecorded retained bytes").expect("write");
    file.sync_all().expect("sync");
    drop(file);
    drop(journal);
    drop(lock);
    fixture
        .store
        .provision_owned(&fixture.identity, &fixture.request)
        .await
        .expect("new recorded birth");
    assert_eq!(
        fs::read(orphan).expect("retained orphan"),
        b"unrecorded retained bytes"
    );
    assert_eq!(fixture.format_count(), 1);
}
#[tokio::test]
async fn contradictory_partial_length_is_held_without_allocation_or_format() {
    let fixture = Fixture::new();
    let mut lock = OwnedJournalLock::acquire(fixture.journal_purpose()).expect("lock");
    let mut journal = OwnedJournal::open(&mut lock).expect("journal");
    let backing = journal.create_staging().expect("birth");
    backing
        .file()
        .set_len(4096)
        .expect("partial contradictory allocation");
    drop(backing);
    drop(journal);
    drop(lock);
    assert!(
        fixture
            .store
            .provision_owned(&fixture.identity, &fixture.request)
            .await
            .is_err()
    );
    assert!(
        fixture
            .store
            .reconcile_owned(&fixture.claim())
            .await
            .is_err()
    );
    assert_eq!(fixture.format_count(), 0);
    assert_eq!(fixture.claim().generation, 1);
}
