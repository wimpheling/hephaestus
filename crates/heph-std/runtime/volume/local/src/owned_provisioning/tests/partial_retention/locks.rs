use super::{Fixture, Stage, Worker, prepare};
use crate::owned_provisioning::partial_retention::PartialBirthCustodyError;
use std::{process::Command, time::Duration};

async fn released_setup_lock(fixture: &Fixture) -> crate::owned_journal::OwnedJournalLock {
    // Another parallel test's fork can briefly inherit the setup descriptor
    // before CLOEXEC closes it. Only this already-released setup acquisition
    // retries; the intentional child/observer contention below must fail fast.
    for _ in 0..100 {
        match fixture.owner.journal_lock(fixture.journal_purpose(), true) {
            Ok(lock) => return lock,
            Err(error) if crate::owned_journal::partial_retention::unavailable(&error) => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(error) => {
                panic!("released setup lock failed without transient contention: {error}")
            }
        }
    }
    panic!("released setup lock remained held after bounded restart retry");
}

fn process_lock(fixture: &Fixture) -> std::process::ExitStatus {
    Command::new("flock")
        .arg("-n")
        .arg(
            fixture
                .root
                .path()
                .join(format!(".{}.lock", fixture.journal_purpose().id())),
        )
        .arg("/bin/true")
        .status()
        .expect("native util-linux flock")
}
#[tokio::test]
async fn concrete_custody_excludes_another_process_across_worker_and_actor_wait() {
    let fixture = Fixture::new();
    prepare(&fixture, Stage::Allocated);
    let worker = Worker::new(&fixture);
    let mut guard = fixture
        .store
        .observe_owned_partial(&worker, &fixture.claim())
        .await
        .unwrap();
    assert_eq!(process_lock(&fixture).code(), Some(1));
    assert!(matches!(
        fixture
            .store
            .observe_owned_partial(&worker, &fixture.claim())
            .await,
        Err(PartialBirthCustodyError::Unavailable)
    ));
    // Model service scheduling between the later worker receipt and actor commit,
    // making no claim that this fixture executes either SQL operation.
    tokio::time::sleep(Duration::from_millis(25)).await;
    guard.revalidate().unwrap();
    assert_eq!(process_lock(&fixture).code(), Some(1));
    drop(guard);
    assert!(process_lock(&fixture).success());
}
#[tokio::test]
async fn actual_formatter_child_custody_makes_partial_observation_unavailable() {
    use crate::owned_journal::OwnedJournal;
    let fixture = Fixture::new();
    prepare(&fixture, Stage::FormatIntent);
    let mut lock = released_setup_lock(&fixture).await;
    let journal = OwnedJournal::open_existing(&mut lock).unwrap();
    let mut child = super::super::super::filesystem::locked_command(
        std::path::Path::new("/bin/sleep"),
        journal.child_lock().unwrap(),
    )
    .arg("30")
    .spawn()
    .unwrap();
    drop(journal);
    drop(lock);
    let worker = Worker::new(&fixture);
    assert!(matches!(
        fixture
            .store
            .observe_owned_partial(&worker, &fixture.claim())
            .await,
        Err(PartialBirthCustodyError::Unavailable)
    ));
    child
        .kill()
        .await
        .expect("terminate and reap actual lock holder");
    let guard = fixture
        .store
        .observe_owned_partial(&worker, &fixture.claim())
        .await
        .unwrap();
    drop(guard);
    assert!(process_lock(&fixture).success());
}
