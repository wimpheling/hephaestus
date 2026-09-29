use super::*;

#[tokio::test]
async fn exchanges_over_exact_ready_instance() {
    let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
    let key = key(Uuid::new_v4(), 1);
    let vm = vm_for(key);
    let (client, mut server) = tokio::io::duplex(16 * 1024);
    vm.push_connection(Box::new(client));
    let (_state_sender, state_receiver) = state();
    registry
        .register(key, vm, state_receiver)
        .expect("registration");
    let mut invalid_policy = policy();
    invalid_policy.max_response_headers = 0;
    assert!(matches!(
        registry.exchange(key, request(), invalid_policy).await,
        Err(GatewayEdgeError::Contract(_))
    ));
    let server_task = tokio::spawn(async move {
        read_headers(&mut server).await;
        server
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .expect("response");
    });
    let response = registry
        .exchange(key, request(), policy())
        .await
        .expect("exchange");
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body, Bytes::from_static(b"ok"));
    drop(registry);
    server_task.await.expect("server task");
}

#[tokio::test]
async fn per_instance_capacity_and_unregister_cancel_are_bounded() {
    let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
    let key = key(Uuid::new_v4(), 1);
    let release = Arc::new(Notify::new());
    let vm = TestVm::blocking(
        VmId(format!("gateway-service-{}", key.identity.instance_id)),
        Arc::clone(&release),
    );
    let (_state_sender, state_receiver) = state();
    registry
        .register(key, vm.clone(), state_receiver)
        .expect("registration");
    let first = tokio::spawn({
        let registry = registry.clone();
        async move { registry.exchange(key, request(), policy()).await }
    });
    vm.opened.notified().await;
    assert!(matches!(
        registry.exchange(key, request(), policy()).await,
        Err(GatewayEdgeError::HandlerUnavailable)
    ));
    registry.unregister(key).expect("unregister");
    assert!(matches!(
        first.await.expect("exchange task"),
        Err(GatewayEdgeError::HandlerUnavailable)
    ));
    release.notify_one();
    drop(registry);
}

#[tokio::test]
async fn caller_cancellation_releases_permit_and_closes_peer() {
    let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
    let key = key(Uuid::new_v4(), 1);
    let vm = vm_for(key);
    let mut first_server = queued_pair(&vm);
    let mut second_server = queued_pair(&vm);
    let (_state_sender, state_receiver) = state();
    registry
        .register(key, vm, state_receiver)
        .expect("registration");
    let (headers_seen, headers_received) = oneshot::channel();
    let (peer_closed, peer_closed_received) = oneshot::channel();
    let first_server_task = tokio::spawn(async move {
        read_headers(&mut first_server).await;
        headers_seen.send(()).expect("headers signal");
        let mut byte = [0_u8; 1];
        let result = first_server.read(&mut byte).await;
        assert!(matches!(result, Ok(0) | Err(_)));
        peer_closed.send(()).expect("close signal");
    });
    let first = tokio::spawn({
        let registry = registry.clone();
        async move { registry.exchange(key, request(), policy()).await }
    });
    headers_received.await.expect("request reached peer");
    first.abort();
    assert!(first.await.expect_err("exchange cancelled").is_cancelled());
    peer_closed_received.await.expect("peer close");
    let second_server_task = tokio::spawn(async move {
        read_headers(&mut second_server).await;
        second_server
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .expect("response");
    });
    let response = registry
        .exchange(key, request(), policy())
        .await
        .expect("permit released");
    assert_eq!(response.body, Bytes::from_static(b"ok"));
    first_server_task.await.expect("first server");
    second_server_task.await.expect("second server");
    drop(registry);
}

#[tokio::test]
async fn stopping_state_cancels_active_exchange_and_closes_peer() {
    let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
    let key = key(Uuid::new_v4(), 1);
    let vm = vm_for(key);
    let mut server = queued_pair(&vm);
    let (state_sender, state_receiver) = state();
    registry
        .register(key, vm, state_receiver)
        .expect("registration");
    let (headers_seen, headers_received) = oneshot::channel();
    let (peer_closed, peer_closed_received) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        read_headers(&mut server).await;
        headers_seen.send(()).expect("headers signal");
        let mut byte = [0_u8; 1];
        let result = server.read(&mut byte).await;
        assert!(matches!(result, Ok(0) | Err(_)));
        peer_closed.send(()).expect("close signal");
    });
    let exchange = tokio::spawn({
        let registry = registry.clone();
        async move { registry.exchange(key, request(), policy()).await }
    });
    headers_received.await.expect("request reached peer");
    state_sender.send_replace(ServiceWorkerState::Stopping);
    assert!(matches!(
        exchange.await.expect("exchange task"),
        Err(GatewayEdgeError::HandlerUnavailable)
    ));
    peer_closed_received.await.expect("peer close");
    server_task.await.expect("server task");
    drop(registry);
}

#[tokio::test]
async fn closed_state_channel_cancels_active_exchange_and_closes_peer() {
    let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
    let key = key(Uuid::new_v4(), 1);
    let vm = vm_for(key);
    let mut server = queued_pair(&vm);
    let (state_sender, state_receiver) = state();
    registry
        .register(key, vm, state_receiver)
        .expect("registration");
    let (headers_seen, headers_received) = oneshot::channel();
    let (peer_closed, peer_closed_received) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        read_headers(&mut server).await;
        headers_seen.send(()).expect("headers signal");
        let mut byte = [0_u8; 1];
        let result = server.read(&mut byte).await;
        assert!(matches!(result, Ok(0) | Err(_)));
        peer_closed.send(()).expect("close signal");
    });
    let exchange = tokio::spawn({
        let registry = registry.clone();
        async move { registry.exchange(key, request(), policy()).await }
    });
    headers_received.await.expect("request reached peer");
    drop(state_sender);
    assert!(matches!(
        exchange.await.expect("exchange task"),
        Err(GatewayEdgeError::HandlerUnavailable)
    ));
    peer_closed_received.await.expect("peer close");
    server_task.await.expect("server task");
    drop(registry);
}

#[tokio::test]
async fn concurrent_exchanges_share_one_ready_vm() {
    let registry = GatewayServiceRegistry::new(1, 2).expect("registry");
    let key = key(Uuid::new_v4(), 1);
    let vm = vm_for(key);
    let mut first_server = queued_pair(&vm);
    let mut second_server = queued_pair(&vm);
    let (_state_sender, state_receiver) = state();
    registry
        .register(key, vm, state_receiver)
        .expect("registration");
    let response_barrier = Arc::new(Barrier::new(2));
    let first_barrier = Arc::clone(&response_barrier);
    let first_server_task = tokio::spawn(async move {
        read_headers(&mut first_server).await;
        first_barrier.wait().await;
        first_server
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\none")
            .await
            .expect("first response");
    });
    let second_barrier = Arc::clone(&response_barrier);
    let second_server_task = tokio::spawn(async move {
        read_headers(&mut second_server).await;
        second_barrier.wait().await;
        second_server
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\ntwo")
            .await
            .expect("second response");
    });
    let first = {
        let registry = registry.clone();
        tokio::spawn(async move { registry.exchange(key, request(), policy()).await })
    };
    let second = {
        let registry = registry.clone();
        tokio::spawn(async move { registry.exchange(key, request(), policy()).await })
    };
    let (first, second) = tokio::time::timeout(Duration::from_secs(1), async {
        (
            first
                .await
                .expect("first exchange")
                .expect("first response"),
            second
                .await
                .expect("second exchange")
                .expect("second response"),
        )
    })
    .await
    .expect("both requests reached the service");
    assert!(
        (first.body == Bytes::from_static(b"one") && second.body == Bytes::from_static(b"two"))
            || (first.body == Bytes::from_static(b"two")
                && second.body == Bytes::from_static(b"one"))
    );
    first_server_task.await.expect("first server");
    second_server_task.await.expect("second server");
    drop(registry);
}

#[tokio::test(start_paused = true)]
async fn total_deadline_covers_delayed_open_and_http_exchange() {
    let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
    let key = key(Uuid::new_v4(), 1);
    let vm = TestVm::delayed(
        VmId(format!("gateway-service-{}", key.identity.instance_id)),
        Duration::from_millis(40),
    );
    let opened = vm.opened.notified();
    let mut server = queued_pair(&vm);
    let (_state_sender, state_receiver) = state();
    registry
        .register(key, vm.clone(), state_receiver)
        .expect("registration");
    let (headers_seen, headers_received) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        read_headers(&mut server).await;
        headers_seen.send(()).expect("headers signal");
        tokio::time::sleep(Duration::from_millis(40)).await;
        let _ = server
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await;
    });
    let mut short = policy();
    short.exchange_timeout = Duration::from_millis(60);
    let exchange = tokio::spawn({
        let registry = registry.clone();
        async move { registry.exchange(key, request(), short).await }
    });
    opened.await;
    tokio::time::advance(Duration::from_millis(40)).await;
    headers_received.await.expect("request reached service");
    tokio::time::advance(Duration::from_millis(20)).await;
    let result = exchange.await.expect("exchange task");
    assert!(matches!(result, Err(GatewayEdgeError::HandlerUnavailable)));
    tokio::time::advance(Duration::from_millis(40)).await;
    server_task.await.expect("server task");
    drop(registry);
}

#[tokio::test]
async fn unregister_keeps_global_slot_until_exchange_quiesces() {
    let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
    let instance_key = key(Uuid::new_v4(), 1);
    let vm = vm_for(instance_key);
    let mut server = queued_pair(&vm);
    let (_state_sender, state_receiver) = state();
    registry
        .register(instance_key, vm, state_receiver)
        .expect("registration");
    let (headers_seen, headers_received) = oneshot::channel();
    let (peer_closed, peer_closed_received) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        read_headers(&mut server).await;
        headers_seen.send(()).expect("headers signal");
        let mut byte = [0_u8; 1];
        let result = server.read(&mut byte).await;
        assert!(matches!(result, Ok(0) | Err(_)));
        peer_closed.send(()).expect("close signal");
    });
    let exchange = tokio::spawn({
        let registry = registry.clone();
        async move { registry.exchange(instance_key, request(), policy()).await }
    });
    headers_received.await.expect("request reached peer");
    registry.unregister(instance_key).expect("unregister");
    let replacement = key(Uuid::new_v4(), 1);
    let (_replacement_sender, replacement_state) = state();
    assert_eq!(
        registry
            .register(replacement, vm_for(replacement), replacement_state,)
            .unwrap_err(),
        GatewayServiceRegistryError::CapacityExhausted
    );
    assert!(matches!(
        exchange.await.expect("exchange task"),
        Err(GatewayEdgeError::HandlerUnavailable)
    ));
    peer_closed_received.await.expect("peer close");
    server_task.await.expect("server task");
    let (_replacement_sender, replacement_state) = state();
    registry
        .register(replacement, vm_for(replacement), replacement_state)
        .expect("slot released after exchange");
    drop(registry);
}
