// Scenario tests intentionally retain Arc and broker handles across awaits so
// concurrent lifecycle behavior remains observable by each task.
#![allow(clippy::significant_drop_tightening)]
use super::support::*;

#[tokio::test]
async fn private_service_dispatch_binds_challenge_to_guest_stream() {
    let temp = TempDir::new().unwrap();
    let runtime_dir = temp.path().join("service-runtime");
    fs::create_dir(&runtime_dir).unwrap();
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let broker = Arc::new(ServiceBroker::bind(&runtime_dir, 1, Duration::from_secs(1)).unwrap());
    let worker = Arc::new(MockWorker::new());
    let mut requests = worker.service_requests.subscribe();
    let instance = service_instance(
        &temp,
        Arc::clone(&worker),
        Duration::from_secs(1),
        Arc::clone(&broker),
        runtime_dir,
    );
    instance.start().await.unwrap();

    let opening = tokio::spawn({
        let instance = Arc::clone(&instance);
        async move { instance.open_private_service_connection().await }
    });
    let request = requests.recv().await.unwrap();
    let mut guest = UnixStream::connect(broker.socket_path()).await.unwrap();
    let mut frame =
        [0_u8; PRIVATE_SERVICE_HANDSHAKE_MAGIC.len() + 1 + 16 + PRIVATE_SERVICE_CHALLENGE_BYTES];
    frame[..PRIVATE_SERVICE_HANDSHAKE_MAGIC.len()]
        .copy_from_slice(&PRIVATE_SERVICE_HANDSHAKE_MAGIC);
    frame[PRIVATE_SERVICE_HANDSHAKE_MAGIC.len()] = PRIVATE_SERVICE_HANDSHAKE_VERSION;
    let id_start = PRIVATE_SERVICE_HANDSHAKE_MAGIC.len() + 1;
    frame[id_start..id_start + 16].copy_from_slice(request.connection_id.as_bytes());
    frame[id_start + 16..].copy_from_slice(request.challenge.as_bytes());
    guest.write_all(&frame).await.unwrap();
    let mut service = opening.await.unwrap().unwrap();
    service.write_all(b"request").await.unwrap();
    let mut request_bytes = [0_u8; 7];
    guest.read_exact(&mut request_bytes).await.unwrap();
    assert_eq!(&request_bytes, b"request");
    broker.shutdown().await;
}
#[tokio::test]
async fn worker_exit_closes_provider_owned_private_service_stream() {
    let temp = TempDir::new().unwrap();
    let runtime_dir = temp.path().join("service-runtime");
    fs::create_dir(&runtime_dir).unwrap();
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let broker = Arc::new(ServiceBroker::bind(&runtime_dir, 1, Duration::from_secs(1)).unwrap());
    let worker = Arc::new(MockWorker::new());
    let mut requests = worker.service_requests.subscribe();
    let instance = service_instance(
        &temp,
        Arc::clone(&worker),
        Duration::from_secs(1),
        Arc::clone(&broker),
        runtime_dir,
    );
    instance.start().await.unwrap();

    let opening = tokio::spawn({
        let instance = Arc::clone(&instance);
        async move { instance.open_private_service_connection().await }
    });
    let request = requests.recv().await.unwrap();
    let mut guest = UnixStream::connect(broker.socket_path()).await.unwrap();
    let mut frame =
        [0_u8; PRIVATE_SERVICE_HANDSHAKE_MAGIC.len() + 1 + 16 + PRIVATE_SERVICE_CHALLENGE_BYTES];
    frame[..PRIVATE_SERVICE_HANDSHAKE_MAGIC.len()]
        .copy_from_slice(&PRIVATE_SERVICE_HANDSHAKE_MAGIC);
    frame[PRIVATE_SERVICE_HANDSHAKE_MAGIC.len()] = PRIVATE_SERVICE_HANDSHAKE_VERSION;
    let id_start = PRIVATE_SERVICE_HANDSHAKE_MAGIC.len() + 1;
    frame[id_start..id_start + 16].copy_from_slice(request.connection_id.as_bytes());
    frame[id_start + 16..].copy_from_slice(request.challenge.as_bytes());
    guest.write_all(&frame).await.unwrap();
    let mut service = opening.await.unwrap().unwrap();

    worker.exit(Some(0), None);
    let mut bytes = [0_u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(1), service.read(&mut bytes))
        .await
        .unwrap();
    assert!(matches!(read, Err(_) | Ok(0)));
}
#[tokio::test]
async fn private_service_timeout_releases_pending_offer() {
    let temp = TempDir::new().unwrap();
    let runtime_dir = temp.path().join("service-runtime");
    fs::create_dir(&runtime_dir).unwrap();
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let broker = Arc::new(ServiceBroker::bind(&runtime_dir, 1, Duration::from_secs(1)).unwrap());
    let instance = service_instance(
        &temp,
        Arc::new(MockWorker::new()),
        Duration::from_millis(20),
        Arc::clone(&broker),
        runtime_dir,
    );
    instance.start().await.unwrap();
    assert!(matches!(
        instance.open_private_service_connection().await,
        Err(VmError::Unavailable { .. })
    ));
    assert!(broker.reserve().is_ok());
    broker.shutdown().await;
}
#[tokio::test]
async fn blocked_private_service_dispatch_is_bounded_and_released() {
    let temp = TempDir::new().unwrap();
    let runtime_dir = temp.path().join("service-runtime");
    fs::create_dir(&runtime_dir).unwrap();
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let broker = Arc::new(ServiceBroker::bind(&runtime_dir, 1, Duration::from_secs(1)).unwrap());
    let worker = Arc::new(MockWorker::blocking_service());
    let instance = service_instance(
        &temp,
        Arc::clone(&worker),
        Duration::from_millis(20),
        Arc::clone(&broker),
        runtime_dir,
    );
    instance.start().await.unwrap();

    let first = tokio::spawn({
        let instance = Arc::clone(&instance);
        async move { instance.open_private_service_connection().await }
    });
    worker.service_entered.notified().await;
    assert!(matches!(
        first.await.unwrap(),
        Err(VmError::Unavailable { .. })
    ));
    assert!(matches!(
        instance.open_private_service_connection().await,
        Err(VmError::Unavailable { .. })
    ));

    worker.block_service.store(false, Ordering::Relaxed);
    worker.release_service.notify_one();
    worker.service_finished.notified().await;
    for _ in 0..16 {
        if instance
            .private_service_dispatch
            .as_ref()
            .expect("service dispatch semaphore")
            .available_permits()
            == 1
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(
        instance
            .private_service_dispatch
            .as_ref()
            .expect("service dispatch semaphore")
            .available_permits(),
        1
    );
    assert!(matches!(
        instance.open_private_service_connection().await,
        Err(VmError::Unavailable { .. })
    ));
    assert!(broker.reserve().is_ok());
    broker.shutdown().await;
}
#[tokio::test(start_paused = true)]
async fn wall_clock_limit_terminates_running_worker() {
    let temp = TempDir::new().unwrap();
    let worker = Arc::new(MockWorker::new());
    let instance = instance_with_wall_clock(&temp, worker, Duration::from_millis(20));
    instance.start().await.unwrap();
    tokio::time::advance(Duration::from_millis(21)).await;
    let exit = instance.wait().await.unwrap();
    assert_eq!(exit.signal, Some(9));
}
#[tokio::test(start_paused = true)]
async fn private_service_survives_wall_clock_limit_until_explicit_destroy() {
    let temp = TempDir::new().unwrap();
    let runtime_dir = temp.path().join("service-runtime");
    fs::create_dir(&runtime_dir).unwrap();
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let broker = Arc::new(ServiceBroker::bind(&runtime_dir, 1, Duration::from_secs(1)).unwrap());
    let control_worker = Arc::new(MockWorker::new());
    let control = instance_with_wall_clock(
        &temp,
        Arc::clone(&control_worker),
        Duration::from_millis(20),
    );
    control.start().await.unwrap();
    let worker = Arc::new(MockWorker::new());
    let instance = service_instance_with_wall_clock(
        &temp,
        Arc::clone(&worker),
        Duration::from_millis(20),
        Duration::from_secs(1),
        Arc::clone(&broker),
        runtime_dir,
    );
    instance.start().await.unwrap();

    // Give both monitor tasks a scheduling turn so the paused-time
    // deadlines are registered before advancing the clock.
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(21)).await;
    let control_exit = control.wait().await.unwrap();
    assert_eq!(control_exit.signal, Some(9));
    assert!(worker.process_exit.borrow().is_none());
    assert!(broker.reserve().is_ok());

    instance.destroy().await.unwrap();
    assert_eq!(
        worker.process_exit.borrow().as_ref().unwrap().signal,
        Some(9)
    );
    assert!(broker.reserve().is_err());
}
