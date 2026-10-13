//! Actual task/spawn/FD controls; no VM/PG/native absence certificate.
use super::super::super::workspace::git_child::GitChild;
use super::super::task::WatcherControl;
use super::*;
#[tokio::test]
async fn known_nonexistent_exec_settles_original_without_child_and_preserves_raw_spawn_error() {
    let fixture = Fixture::new(Mode::Verify);
    let node = fixture.node.clone();
    let missing = fixture
        .temporary
        .path()
        .join("actual-executable-does-not-exist");
    jobs::start(&fixture.node, async move {
        let mut command = std::process::Command::new(missing);
        let error = match GitChild::spawn(&mut command, &node) {
            Ok(_) => panic!("nonexistent executable unexpectedly admitted"),
            Err(error) => error,
        };
        Outcome::Verify(Err(Arc::new(error)))
    });
    bounded(observe(&[fixture.node.clone()], Duration::from_secs(1)))
        .await
        .unwrap();
    assert!(fixture.node.settled());
    assert!(!fixture.path().exists());
    assert_eq!(fixture.token.provisions.load(Ordering::SeqCst), 0);
    assert!(lock(&fixture.node.state).errors.iter().any(|error| {
        matches!(error.downcast_ref::<super::super::BuildExecutionError>(),
            Some(super::super::BuildExecutionError::FilesystemSource(source))
                if source.kind() == std::io::ErrorKind::NotFound)
    }));
}
#[tokio::test]
async fn terminated_outer_watcher_and_cleanup_are_not_settled_until_actual_joins() {
    let fixture = Fixture::new(Mode::Verify);
    let control = WatcherControl::new(true);
    *lock(&fixture.node.watcher_control) = Some(control.clone());
    jobs::start(&fixture.node, async {
        Outcome::Verify(Err(Arc::new(super::super::BuildExecutionError::Release)))
    });
    bounded(control.entered.notified()).await;
    // Inner work has actually joined. The same outer watcher remains alive.
    assert!(lock(&fixture.node.work_job).as_ref().unwrap().joined());
    fixture.node.close();
    fixture.node.kick_cleanup(true);
    let original_cleanup = lock(&fixture.node.cleanup_job).as_ref().unwrap().clone();
    bounded(async {
        while !original_cleanup.ready() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(fixture.pin.removed()); // Actual positive physical/workspace cleanup.
    assert!(!original_cleanup.joined());
    fixture.node.kick_cleanup(true);
    assert!(Arc::ptr_eq(
        &original_cleanup,
        lock(&fixture.node.cleanup_job).as_ref().unwrap()
    ));
    assert!(!fixture.node.settled());
    assert!(
        observe(&[fixture.node.clone()], Duration::from_millis(20))
            .await
            .is_err()
    );
    assert!(original_cleanup.joined());
    let original_watcher = lock(&fixture.node.execution_job).as_ref().unwrap().clone();
    control.released.notify_one();
    bounded(async {
        while !original_watcher.ready() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(!original_watcher.joined());
    assert!(!fixture.node.settled()); // is_finished/panic is NOT an actual join.
    fixture.node.join_finished_jobs().await;
    assert!(original_watcher.joined());
    assert!(fixture.node.settled());
    assert!(lock(&fixture.node.state).errors.iter().any(|error| {
        error
            .downcast_ref::<tokio::task::JoinError>()
            .is_some_and(tokio::task::JoinError::is_panic)
    }));
    assert!(lock(&fixture.node.state).errors.iter().any(|error| {
        matches!(
            error.downcast_ref::<super::super::BuildExecutionError>(),
            Some(super::super::BuildExecutionError::Release)
        )
    }));
    // Historical panic does not permanently deny a genuine positive retry.
    fixture.node.kick_cleanup(true);
    bounded(observe(&[fixture.node.clone()], Duration::from_secs(1)))
        .await
        .unwrap();
    assert!(!fixture.path().exists());
}
