// Scenario tests intentionally retain Arc and broker handles across awaits so
// concurrent lifecycle behavior remains observable by each task.
#![allow(clippy::significant_drop_tightening)]
use super::support::*;

#[tokio::test]
async fn concurrent_start_is_shared_and_events_are_ordered() {
    let temp = TempDir::new().unwrap();
    let worker = Arc::new(MockWorker::new());
    let instance = instance(&temp, worker.clone());
    let mut events = instance.subscribe_events();

    let first = Arc::clone(&instance);
    let second = Arc::clone(&instance);
    let (first, second) = tokio::join!(first.start(), second.start());
    first.unwrap();
    second.unwrap();
    assert_eq!(worker.start_calls.load(Ordering::Relaxed), 1);
    assert!(matches!(
        events.recv().await.unwrap(),
        VmEvent::Started { .. }
    ));
    assert!(matches!(events.recv().await.unwrap(), VmEvent::Ready));
}
#[tokio::test]
async fn many_waiters_receive_one_cached_exit() {
    let temp = TempDir::new().unwrap();
    let worker = Arc::new(MockWorker::new());
    let instance = instance(&temp, worker);
    instance.start().await.unwrap();

    let first = Arc::clone(&instance);
    let second = Arc::clone(&instance);
    let (first, second, stopped) = tokio::join!(
        first.wait(),
        second.wait(),
        instance.stop(StopMode::Graceful {
            timeout: Duration::from_secs(1),
        })
    );
    stopped.unwrap();
    assert_eq!(first.unwrap(), second.unwrap());
    assert_eq!(instance.wait().await.unwrap().code, Some(0));
}
#[tokio::test]
async fn destroy_before_start_is_idempotent_and_typed() {
    let temp = TempDir::new().unwrap();
    let worker = Arc::new(MockWorker::new());
    let instance = instance(&temp, worker);
    instance.destroy().await.unwrap();
    instance.destroy().await.unwrap();
    assert!(matches!(instance.wait().await, Err(VmError::Destroyed)));
}
#[tokio::test]
async fn concurrent_start_callers_share_a_typed_failure() {
    let temp = TempDir::new().unwrap();
    let worker = Arc::new(MockWorker::failing_start());
    let instance = instance(&temp, worker.clone());
    let first = Arc::clone(&instance);
    let second = Arc::clone(&instance);
    let (first, second) = tokio::join!(first.start(), second.start());
    assert!(matches!(first, Err(VmError::Unavailable { .. })));
    assert!(matches!(second, Err(VmError::Unavailable { .. })));
    assert_eq!(worker.start_calls.load(Ordering::Relaxed), 1);
    assert!(matches!(
        instance.start().await,
        Err(VmError::Unavailable { .. })
    ));
}
#[tokio::test]
async fn destroy_during_startup_does_not_resurrect_instance() {
    let temp = TempDir::new().unwrap();
    let worker = Arc::new(MockWorker::blocking_start());
    let instance = instance(&temp, worker.clone());
    let start_instance = Arc::clone(&instance);
    let started = tokio::spawn(async move { start_instance.start().await });
    worker.start_entered.notified().await;

    instance.destroy().await.unwrap();
    worker.release_start.notify_one();

    let start_result = started.await.unwrap();
    assert!(
        matches!(start_result, Err(VmError::Destroyed)),
        "start completed with {start_result:?}"
    );
    assert!(matches!(instance.start().await, Err(VmError::Destroyed)));
    instance.destroy().await.unwrap();
}
#[tokio::test]
async fn private_http_response_is_correlated_to_its_waiter() {
    let temp = TempDir::new().unwrap();
    let worker = Arc::new(MockWorker::new());
    let instance = instance(&temp, worker);
    instance.start().await.unwrap();
    let response = instance
        .invoke_private_http(PrivateHttpRequest {
            method: http::Method::POST,
            path_and_query: "/gateway/echo".to_owned(),
            headers: http::HeaderMap::new(),
            body: bytes::Bytes::from_static(b"request"),
        })
        .await
        .unwrap();
    assert_eq!(response.status, http::StatusCode::CREATED);
    assert_eq!(response.body, bytes::Bytes::from_static(b"response"));
}
#[tokio::test]
async fn worker_crash_without_exit_event_is_cached_and_forwarded_once() {
    let temp = TempDir::new().unwrap();
    let worker = Arc::new(MockWorker::new());
    let instance = instance(&temp, worker.clone());
    let mut events = instance.subscribe_events();
    instance.start().await.unwrap();
    assert!(matches!(
        events.recv().await.unwrap(),
        VmEvent::Started { .. }
    ));
    assert!(matches!(events.recv().await.unwrap(), VmEvent::Ready));

    worker.crash(11);
    let exit = instance.wait().await.unwrap();
    assert_eq!(exit.signal, Some(11));
    match events.recv().await.unwrap() {
        VmEvent::Exited(event_exit) => assert_eq!(event_exit, exit),
        event => panic!("expected terminal event, received {event:?}"),
    }
    assert_eq!(instance.wait().await.unwrap(), exit);
    assert!(
        tokio::time::timeout(Duration::from_millis(25), events.recv())
            .await
            .is_err()
    );
    instance.destroy().await.unwrap();
}
#[tokio::test(start_paused = true)]
async fn readiness_timeout_is_typed_and_force_cleans_worker() {
    let temp = TempDir::new().unwrap();
    let worker = Arc::new(MockWorker::without_ready());
    let instance = instance(&temp, worker);
    let starting_instance = Arc::clone(&instance);
    let starting = tokio::spawn(async move { starting_instance.start().await });
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(21)).await;
    assert!(matches!(
        starting.await.unwrap(),
        Err(VmError::Unavailable { resource, .. }) if resource == "guest readiness"
    ));
    assert!(instance.wait().await.is_ok());
}
#[tokio::test]
async fn cached_exit_survives_lagged_log_subscriber() {
    let temp = TempDir::new().unwrap();
    let worker = Arc::new(MockWorker::new());
    let instance = instance(&temp, worker.clone());
    let mut slow_events = instance.subscribe_events();
    instance.start().await.unwrap();
    for index in 0..64 {
        drop(worker.events.send(WorkerEvent::Log {
            stream: crate::worker::WireLogStream::Stdout,
            bytes: index.to_string().into_bytes(),
        }));
        tokio::task::yield_now().await;
    }
    worker.exit(Some(23), None);
    assert_eq!(instance.wait().await.unwrap().code, Some(23));
    assert!(matches!(
        slow_events.recv().await,
        Err(broadcast::error::RecvError::Lagged(_))
    ));
    assert_eq!(instance.wait().await.unwrap().code, Some(23));
}
