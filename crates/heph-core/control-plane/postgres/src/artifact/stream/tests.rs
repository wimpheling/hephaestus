use super::{ArtifactCancellation, Boundary, run_with_boundary};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    sync::{Notify, mpsc},
    time::timeout,
};

#[derive(Clone)]
struct TestCancellation {
    canceled: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl TestCancellation {
    fn new() -> Self {
        Self {
            canceled: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
        }
    }
}

impl ArtifactCancellation for TestCancellation {
    fn is_cancelled(&self) -> bool {
        self.canceled.load(Ordering::SeqCst)
    }

    fn cancel(&self) {
        self.canceled.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    fn cancelled<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(self.notify.notified())
    }
}

#[tokio::test]
async fn receiver_drop_cancels_pending_file_operation() {
    let cancellation = TestCancellation::new();
    let (sender, receiver) = mpsc::channel(1);
    drop(receiver);

    let result =
        run_with_boundary(&cancellation, &sender, None, std::future::pending::<()>()).await;

    assert_eq!(result, Err(Boundary::Closed));
    assert!(cancellation.is_cancelled());
}

#[tokio::test]
async fn absolute_deadline_stops_pending_file_operation() {
    let cancellation = TestCancellation::new();
    let (sender, _receiver) = mpsc::channel(1);
    let result = timeout(
        Duration::from_millis(50),
        run_with_boundary(
            &cancellation,
            &sender,
            Some(Instant::now() + Duration::from_millis(1)),
            std::future::pending::<()>(),
        ),
    )
    .await
    .expect("absolute deadline should win");

    assert_eq!(result, Err(Boundary::Deadline));
    assert!(cancellation.is_cancelled());
}

#[tokio::test]
async fn expired_deadline_wins_before_ready_operation() {
    let cancellation = TestCancellation::new();
    let (sender, _receiver) = mpsc::channel(1);
    let result = run_with_boundary(
        &cancellation,
        &sender,
        Some(Instant::now() - Duration::from_millis(1)),
        std::future::ready(()),
    )
    .await;

    assert_eq!(result, Err(Boundary::Deadline));
    assert!(cancellation.is_cancelled());
}

#[tokio::test]
async fn existing_cancellation_wins_before_ready_operation() {
    let cancellation = TestCancellation::new();
    cancellation.cancel();
    let (sender, _receiver) = mpsc::channel(1);
    let result = run_with_boundary(&cancellation, &sender, None, std::future::ready(())).await;

    assert_eq!(result, Err(Boundary::Canceled));
}
