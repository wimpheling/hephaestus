//! Controlled production ownership/scheduling, not PG/native receipt evidence.
mod custody;
mod finalization;
mod git_child;
mod settlement;
mod support;
use super::{
    cleanup::observe,
    jobs, lock,
    models::{Mode, Outcome},
    registry::Admission,
};
use support::*;
use vm_trait::VmProvisionAttempt;

#[tokio::test]
async fn close_detaches_while_database_job_is_stalled_and_waiter_abort_keeps_original() {
    let fixture = Fixture::new(Mode::Execute);
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let node = fixture.node.clone();
    let token = fixture.token.clone();
    jobs::start(&fixture.node, {
        let entered = entered.clone();
        let release = release.clone();
        async move {
            node.install_attempt(token.clone()).unwrap();
            let instance = token.provision().await.unwrap();
            node.instance(instance);
            entered.notify_one();
            release.notified().await;
            failure()
        }
    });
    bounded(entered.notified()).await;
    let waiter = tokio::spawn({
        let node = fixture.node.clone();
        async move { node.outcome().await }
    });
    waiter.abort();
    assert!(bounded(waiter).await.err().unwrap().is_cancelled());
    fixture.registry.close();
    fixture.node.kick_cleanup(true);
    bounded(fixture.token.instance.entered.notified()).await;
    assert!(fixture.token.instance.destroyed.load(Ordering::SeqCst));
    assert!(
        observe(&[fixture.node.clone()], Duration::from_millis(20))
            .await
            .is_err()
    );
    assert!(fixture.path().exists()); // Still in use by actual execution task.
    release.notify_one();
    bounded(observe(&[fixture.node.clone()], Duration::from_secs(1)))
        .await
        .unwrap();
    assert!(!fixture.path().exists());
    assert_eq!(fixture.token.provisions.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn start_and_detach_failure_keep_raw_originals_workspace_and_deny_retry_reset() {
    let fixture = Fixture::new(Mode::Execute);
    fixture
        .token
        .instance
        .destroy_errors
        .store(1, Ordering::SeqCst);
    let node = fixture.node.clone();
    let token = fixture.token.clone();
    jobs::start(&fixture.node, async move {
        node.install_attempt(token.clone()).unwrap();
        let instance = token.provision().await.unwrap();
        node.instance(instance.clone());
        node.vm_error(instance.start().await.unwrap_err());
        let error = node.detach().await.unwrap_err();
        Outcome::Build(Err(Arc::new(error)))
    });
    fixture.node.outcome().await;
    watcher_finished(&fixture.node).await;
    assert!(fixture.path().exists());
    assert!(!fixture.token.instance.destroyed.load(Ordering::SeqCst));
    assert!(
        lock(&fixture.node.state)
            .errors
            .iter()
            .filter(|error| error.downcast_ref::<vm_trait::VmError>().is_some())
            .count()
            >= 2
    );
    assert!(matches!(
        fixture.registry.register(fixture.node.id, Mode::Retry),
        Err(super::BuildExecutionError::VmCleanup)
    ));
    assert!(matches!(
        fixture.registry.register(fixture.node.id, Mode::Verify),
        Err(super::BuildExecutionError::VmCleanup)
    ));
    fixture.node.close();
    fixture.node.kick_cleanup(true);
    bounded(observe(&[fixture.node.clone()], Duration::from_secs(1)))
        .await
        .unwrap();
    assert_eq!(fixture.token.provisions.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.token.cleanups.load(Ordering::SeqCst), 2);
    assert!(!fixture.path().exists());
}
#[tokio::test]
async fn admission_close_is_monotonic_and_same_id_verification_cannot_overlap() {
    let fixture = Fixture::new(Mode::Execute);
    assert!(matches!(
        fixture.registry.register(fixture.node.id, Mode::Execute),
        Ok(Admission::Follow(_))
    ));
    assert!(matches!(
        fixture.registry.register(fixture.node.id, Mode::Verify),
        Err(super::BuildExecutionError::AlreadyClaimed)
    ));
    fixture.registry.close();
    assert!(matches!(
        fixture
            .registry
            .register(BuildRequestId::new(), Mode::Execute),
        Err(super::BuildExecutionError::Closed)
    ));
    assert!(matches!(
        fixture
            .registry
            .register(BuildRequestId::new(), Mode::Retry),
        Err(super::BuildExecutionError::Closed)
    ));
    // This node has no actual execution job: closed intake alone is NEVER a
    // settlement claim. Original registration remains held.
    assert!(!fixture.node.settled());
}
