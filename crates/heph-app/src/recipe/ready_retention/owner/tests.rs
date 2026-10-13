//! Controlled Tokio/OFD ownership schedules, NOT Local/PG/ext4 custody proof.
//! Production's concrete guard is opaque; these controls use an actual file FD
//! through the same detach primitive and explicitly labelled channel schedules.
use super::super::ReadyRetentionError;
use super::detach;
use std::{
    fs::File,
    os::unix::fs::MetadataExt,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

struct OriginalFile {
    file: File,
    drops: Arc<AtomicUsize>,
}
impl Drop for OriginalFile {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
fn original() -> (OriginalFile, Arc<AtomicUsize>) {
    let drops = Arc::new(AtomicUsize::new(0));
    (
        OriginalFile {
            file: tempfile::tempfile().unwrap(),
            drops: drops.clone(),
        },
        drops,
    )
}
async fn bounded<T>(receiver: oneshot::Receiver<T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), receiver)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn aborted_reply_waiter_does_not_cancel_registered_probe_owner() {
    let (owner, drops) = original();
    let inode = owner.file.metadata().unwrap().ino();
    let (release, pause) = oneshot::channel();
    let (reply, completed) = oneshot::channel();
    let (settled, observed) = oneshot::channel();
    detach(async move {
        bounded(pause).await;
        assert_eq!(owner.file.metadata().unwrap().ino(), inode);
        let _ = reply.send(());
        drop(owner);
        settled.send(()).unwrap();
    });
    let waiter = tokio::spawn(async move {
        let _ = completed.await;
    });
    waiter.abort(); // Only this disposable caller/waiter has an abort handle.
    let _ = waiter.await;
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    release.send(()).unwrap();
    bounded(observed).await;
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn uncertain_worker_then_actor_controls_keep_same_original_file_and_error() {
    let (owner, drops) = original();
    let inode = owner.file.metadata().unwrap().ino();
    let error = Arc::new(ReadyRetentionError::CompletionUnknown);
    let expected_error = error.clone();
    let (requests, mut retries) = mpsc::channel::<()>(1);
    let (reply, failed) = oneshot::channel();
    let (next, second) = oneshot::channel();
    let (settled, observed) = oneshot::channel();
    detach(async move {
        let mut held_errors = vec![error.clone()];
        reply.send(error).unwrap(); // Worker reply is unknown, not NotApplied.
        assert!(retries.recv().await.is_some());
        assert_eq!(owner.file.metadata().unwrap().ino(), inode);
        held_errors.push(Arc::new(ReadyRetentionError::Correlation));
        next.send(held_errors[0].clone()).unwrap(); // Actor control also fails.
        assert!(retries.recv().await.is_some());
        assert_eq!(owner.file.metadata().unwrap().ino(), inode);
        assert_eq!(held_errors.len(), 2);
        drop(owner); // Controlled release only, no simulated durable completion.
        settled.send(()).unwrap();
    });
    let actual = bounded(failed).await;
    assert!(Arc::ptr_eq(&actual, &expected_error));
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    requests.send(()).await.unwrap();
    let actual = bounded(second).await;
    assert!(Arc::ptr_eq(&actual, &expected_error));
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    requests.send(()).await.unwrap();
    bounded(observed).await;
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn one_receiver_serializes_retry_while_original_pass_is_inflight() {
    let (owner, drops) = original();
    let (requests, mut receiver) = mpsc::channel(1);
    let (release, paused) = oneshot::channel();
    let (started, first) = oneshot::channel();
    let (settled, observed) = oneshot::channel();
    let passes = Arc::new(AtomicUsize::new(0));
    let observed_passes = passes.clone();
    detach(async move {
        assert_eq!(receiver.recv().await, Some(1));
        passes.fetch_add(1, Ordering::SeqCst);
        started.send(()).unwrap();
        bounded(paused).await;
        assert_eq!(receiver.recv().await, Some(2));
        passes.fetch_add(1, Ordering::SeqCst);
        drop(owner);
        settled.send(()).unwrap();
    });
    requests.send(1).await.unwrap();
    bounded(first).await;
    requests.send(2).await.unwrap();
    assert_eq!(observed_passes.load(Ordering::SeqCst), 1);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    release.send(()).unwrap();
    bounded(observed).await;
    assert_eq!(observed_passes.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn closed_intake_is_acyclic_and_cannot_cancel_inflight_file_owner() {
    let (owner, drops) = original();
    let (requests, mut receiver) = mpsc::channel::<()>(1);
    let (release, paused) = oneshot::channel();
    let (settled, observed) = oneshot::channel();
    detach(async move {
        bounded(paused).await;
        assert!(receiver.recv().await.is_none()); // No task-owned sender cycle.
        assert!(owner.file.metadata().is_ok());
        drop(owner); // Explicit owner loss has no cleanup/absence guarantee.
        settled.send(()).unwrap();
    });
    drop(requests);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    release.send(()).unwrap();
    bounded(observed).await;
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
