//! Actual Tokio/channel ownership controls; no real provider or PG receipt proof.
use super::super::{Mode, tests::request};
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct OriginalOwner(Arc<AtomicUsize>);
impl Drop for OriginalOwner {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn abandoning_reply_waiter_does_not_abort_detached_original_owner() {
    let drops = Arc::new(AtomicUsize::new(0));
    let owner = OriginalOwner(drops.clone());
    let (release, paused) = oneshot::channel();
    let (reply, completed) = oneshot::channel::<()>();
    let (settled, observed) = oneshot::channel();
    detach(async move {
        paused.await.unwrap();
        let _ = reply.send(()); // The absent caller is not a task cancellation signal.
        drop(owner);
        settled.send(()).unwrap();
    });
    let waiter = tokio::spawn(async move {
        let _ = completed.await;
    });
    waiter.abort(); // Abort ONLY the reply waiter; no production abort handle exists.
    let _ = waiter.await;
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), observed)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn retry_channel_preserves_registration_and_current_request_without_new_owner() {
    let original_request = request(9, Mode::Original);
    let key = (
        original_request.remove.deployment_id,
        original_request.resource.clone(),
    );
    let registry = Arc::new(Mutex::new(BTreeMap::new()));
    let (initial, receiver) = register(&registry, &key, &original_request);
    let mut receiver = receiver.unwrap();
    let original = admission::Original::new(original_request.clone());
    let failure = Arc::new(PreparedInstanceDrainError::Pending);
    publish(
        &Arc::downgrade(&registry),
        &key,
        &original,
        &Err(failure.clone()),
    );
    let fresh = request(11, Mode::Resume);
    let (retry, new_owner) = register(&registry, &key, &fresh);
    assert!(new_owner.is_none());
    let (result, reply) = oneshot::channel();
    drop(reply);
    assert!(
        retry
            .send(Delivery {
                request: fresh.clone(),
                result
            })
            .await
            .is_ok()
    );
    let delivered = receiver.recv().await.unwrap();
    admission::compare_request(&delivered.request, &original).unwrap();
    assert_eq!(delivered.request.identity, fresh.identity);
    assert_eq!(original.request.remove, original_request.remove);
    let status = inspect(&registry);
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].original_command, original_request.remove.command);
    assert!(status[0].held);
    drop(delivered);
    drop(retry);
    drop(initial);
    drop(receiver);
}

#[tokio::test]
async fn closed_intake_has_no_sender_cycle_and_owns_inflight_work_until_settled() {
    let original = request(9, Mode::Original);
    let key = (original.remove.deployment_id, original.resource.clone());
    let registry = Arc::new(Mutex::new(BTreeMap::new()));
    let weak = Arc::downgrade(&registry);
    let (sender, receiver) = register(&registry, &key, &original);
    let mut receiver = receiver.unwrap();
    drop(sender);
    let drops = Arc::new(AtomicUsize::new(0));
    let owner = OriginalOwner(drops.clone());
    let (release, paused) = oneshot::channel();
    let (settled, observed) = oneshot::channel();
    detach(async move {
        paused.await.unwrap();
        assert!(receiver.recv().await.is_none());
        assert!(weak.upgrade().is_none());
        drop(owner); // No physical absence/cleanup claim follows last owner Drop.
        settled.send(()).unwrap();
    });
    drop(registry);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), observed)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
