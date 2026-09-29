use std::sync::{Arc, atomic::Ordering};

use run_orchestrator::{CompositeRunCompletionObserver, RunCompletionObserver};

use super::support::{CountingCompletion, test_run};

#[tokio::test]
async fn composite_completion_runs_every_observer_and_sums_recovery() {
    let first = Arc::new(CountingCompletion::new(2));
    let second = Arc::new(CountingCompletion::new(3));
    let observer = CompositeRunCompletionObserver::new(vec![first.clone(), second.clone()]);
    let run = test_run();

    observer
        .after_cleanup(&run)
        .await
        .expect("completion observers should run");
    assert_eq!(first.completions.load(Ordering::SeqCst), 1);
    assert_eq!(second.completions.load(Ordering::SeqCst), 1);
    assert_eq!(
        observer.recover().await.expect("recovery should succeed"),
        5
    );
}
