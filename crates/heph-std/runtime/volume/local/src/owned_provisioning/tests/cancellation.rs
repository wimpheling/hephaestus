use super::support::Fixture;
use crate::owned_journal::{JournalError, OwnedJournalLock};
use std::{fs, time::Duration};

#[tokio::test]
async fn cancelled_formatter_retains_volume_flock_until_child_exits() {
    let fixture = Fixture::new();
    // The shell inherits the lock and writes its PID before any real format.
    // exec ensures the actual child holding the descriptor is the sleep process.
    let pid_file = fixture.tools.path().join("formatter-pid");
    fixture.formatter(&format!(
        "printf '%s' \"$$\" > '{}'\nexec /bin/sleep 60\n",
        pid_file.display()
    ));
    let store = fixture.store.clone();
    let identity = fixture.identity.clone();
    let request = fixture.request.clone();
    let task = tokio::spawn(async move { store.provision_owned(&identity, &request).await });
    let mut started = false;
    for _ in 0..200 {
        if pid_file.exists() {
            started = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if !started {
        task.abort();
    }
    assert!(started, "formatter child must start");
    assert!(
        matches!(
            OwnedJournalLock::acquire_existing(fixture.journal_purpose()),
            Err(JournalError::Locked)
        ),
        "actual formatter holds same-volume exclusion"
    );
    task.abort();
    assert!(task.await.expect_err("cancelled task").is_cancelled());
    // kill_on_drop requests termination; release is legal only after the child's
    // inherited flock actually closes. This bounded retry does not change production.
    let mut released = false;
    for _ in 0..200 {
        match OwnedJournalLock::acquire_existing(fixture.journal_purpose()) {
            Ok(lock) => {
                drop(lock);
                released = true;
                break;
            }
            Err(JournalError::Locked) => tokio::time::sleep(Duration::from_millis(10)).await,
            Err(error) => panic!("unexpected recovery failure: {error}"),
        }
    }
    assert!(released, "actual child eventually releases exclusion");
    assert!(
        fixture
            .store
            .reconcile_owned(&fixture.claim())
            .await
            .is_err()
    );
    assert_eq!(fixture.format_count(), 1);
    assert_eq!(
        fs::metadata(fixture.canonical())
            .expect("retained inode")
            .len(),
        16 * 1024 * 1024
    );
}

#[tokio::test]
async fn inherited_formatter_lock_survives_parent_journal_drop_until_child_reaped() {
    let fixture = Fixture::new();
    let mut lock = OwnedJournalLock::acquire(fixture.journal_purpose()).expect("lock");
    let mut journal = crate::owned_journal::OwnedJournal::open(&mut lock).expect("journal");
    let backing = journal.create_staging().expect("birth");
    backing.file().set_len(16 * 1024 * 1024).expect("allocate");
    journal.mark_allocated(&backing).expect("allocation record");
    journal.publish(&backing).expect("publication");
    journal
        .begin_first_format(&backing)
        .expect("durable format intent");
    let mut child = super::super::filesystem::locked_command(
        std::path::Path::new("/bin/sleep"),
        journal.child_lock().expect("inherited lock"),
    )
    .arg("60")
    .spawn()
    .expect("formatter child");
    drop(backing);
    drop(journal);
    drop(lock);
    assert!(
        matches!(
            OwnedJournalLock::acquire_existing(fixture.journal_purpose()),
            Err(JournalError::Locked)
        ),
        "actual inherited flock survives parent descriptor drop"
    );
    child.kill().await.expect("terminate and reap formatter");
    let released = OwnedJournalLock::acquire_existing(fixture.journal_purpose())
        .expect("reaped child releases lock");
    drop(released);
    assert!(
        fixture
            .store
            .reconcile_owned(&fixture.claim())
            .await
            .is_err()
    );
    assert_eq!(
        fixture.format_count(),
        0,
        "readonly recovery never repeats durable format intent"
    );
}
