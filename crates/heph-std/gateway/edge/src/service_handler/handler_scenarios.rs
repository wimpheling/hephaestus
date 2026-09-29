use super::*;

#[tokio::test]
async fn stateless_target_delegates_without_service_exchange() {
    let route = route(Duration::from_secs(1));
    let stateless = Stateless {
        calls: AtomicUsize::new(0),
    };
    let resolver = Resolver {
        target: Ok(GatewayExecutionTarget::Stateless),
        delay: Duration::ZERO,
        calls: AtomicUsize::new(0),
    };
    let handler = GatewayServiceHandler::new(
        resolver,
        stateless,
        GatewayServiceRegistry::new(1, 1).expect("registry"),
        owner(),
    )
    .expect("handler");
    let response = handler
        .invoke(&route, Uuid::new_v4(), request())
        .await
        .expect("stateless response");
    assert_eq!(response.status, StatusCode::NO_CONTENT);
    assert_eq!(handler.stateless.calls.load(Ordering::Relaxed), 1);
    drop(handler);
}

#[tokio::test]
async fn service_target_uses_exact_registry_key_and_preserves_application_headers() {
    let route = route(Duration::from_secs(1));
    let (registry, key, captured) =
        service_fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
    let stateless = Stateless {
        calls: AtomicUsize::new(0),
    };
    let resolver = Resolver {
        target: Ok(GatewayExecutionTarget::Service(budget(
            key,
            Duration::from_secs(1),
        ))),
        delay: Duration::ZERO,
        calls: AtomicUsize::new(0),
    };
    let handler =
        GatewayServiceHandler::new(resolver, stateless, registry, owner()).expect("handler");
    let mut inbound = request();
    inbound
        .headers
        .insert("x-application", HeaderValue::from_static("kept"));
    let response = handler
        .invoke(&route, Uuid::new_v4(), inbound)
        .await
        .expect("service response");
    assert_eq!(response.status, StatusCode::OK);
    let wire = captured.await.expect("captured request");
    let wire = String::from_utf8(wire).expect("HTTP bytes");
    assert!(wire.contains("x-application: kept"));
    assert!(wire.contains("authorization: Bearer rewritten-application-secret"));
    assert_eq!(wire.matches("authorization:").count(), 1);
    assert!(!wire.to_ascii_lowercase().contains("x-platform"));
    assert_eq!(handler.stateless.calls.load(Ordering::Relaxed), 0);
    drop(handler);
}

#[tokio::test]
async fn missing_or_stale_service_target_does_not_fall_back() {
    let route = route(Duration::from_secs(1));
    let (registry, registered_key, _captured) =
        service_fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let stale_key = GatewayServiceInstanceKey {
        identity: registered_key.identity,
        fencing_token: registered_key.fencing_token + 1,
    };
    let stateless = Stateless {
        calls: AtomicUsize::new(0),
    };
    let resolver = Resolver {
        target: Ok(GatewayExecutionTarget::Service(budget(
            stale_key,
            Duration::from_secs(1),
        ))),
        delay: Duration::ZERO,
        calls: AtomicUsize::new(0),
    };
    let handler =
        GatewayServiceHandler::new(resolver, stateless, registry, owner()).expect("handler");
    let result = handler.invoke(&route, Uuid::new_v4(), request()).await;
    assert!(result.is_err());
    assert_eq!(handler.stateless.calls.load(Ordering::Relaxed), 0);
    drop(handler);
}

#[tokio::test]
async fn resolver_failure_does_not_fall_back() {
    let route = route(Duration::from_secs(1));
    let stateless = Stateless {
        calls: AtomicUsize::new(0),
    };
    let handler = GatewayServiceHandler::new(
        Resolver {
            target: Err(GatewayExecutionTargetError::Unavailable),
            delay: Duration::ZERO,
            calls: AtomicUsize::new(0),
        },
        stateless,
        GatewayServiceRegistry::new(1, 1).expect("registry"),
        owner(),
    )
    .expect("handler");
    assert!(
        handler
            .invoke(&route, Uuid::new_v4(), request())
            .await
            .is_err()
    );
    assert_eq!(handler.stateless.calls.load(Ordering::Relaxed), 0);
    drop(handler);
}

#[test]
fn constructor_rejects_invalid_owner() {
    let invalid = GatewayServiceOwner {
        host_id: String::new(),
        owner_uuid: Uuid::new_v4(),
    };
    let result = GatewayServiceHandler::new(
        Resolver {
            target: Ok(GatewayExecutionTarget::Stateless),
            delay: Duration::ZERO,
            calls: AtomicUsize::new(0),
        },
        Stateless {
            calls: AtomicUsize::new(0),
        },
        GatewayServiceRegistry::new(1, 1).expect("registry"),
        invalid,
    );
    assert!(matches!(
        result,
        Err(GatewayEdgeError::InvalidRoute("invalid service owner"))
    ));
    drop(result);
}

#[tokio::test(start_paused = true)]
async fn lookup_and_exchange_share_one_route_deadline() {
    let route = route(Duration::from_millis(50));
    let (registry, key, _captured) = service_fixture_with_delay(
        b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        Duration::from_millis(30),
    );
    let stateless = Stateless {
        calls: AtomicUsize::new(0),
    };
    let resolver = Resolver {
        target: Ok(GatewayExecutionTarget::Service(budget(
            key,
            Duration::from_secs(1),
        ))),
        delay: Duration::from_millis(30),
        calls: AtomicUsize::new(0),
    };
    let handler =
        GatewayServiceHandler::new(resolver, stateless, registry, owner()).expect("handler");
    let invocation =
        tokio::spawn(async move { handler.invoke(&route, Uuid::new_v4(), request()).await });
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(30)).await;
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(20)).await;
    assert!(invocation.await.expect("lookup task").is_err());
}

#[tokio::test]
// The registry is deliberately moved into the handler and retained until
// the cancellation stream has observed its exchange cleanup.
#[allow(clippy::significant_drop_tightening)]
async fn session_budget_cancellation_closes_service_exchange() {
    let route = route(Duration::from_secs(1));
    let key = key();
    let (host, mut peer) = tokio::io::duplex(4096);
    let (closed, closed_receiver) = oneshot::channel();
    tokio::spawn(async move {
        let mut bytes = [0_u8; 512];
        let _ = peer.read(&mut bytes).await;
        let mut rest = Vec::new();
        let _ = peer.read_to_end(&mut rest).await;
        let _ = closed.send(());
    });
    let (state_sender, state) = watch::channel(ServiceWorkerState::Ready);
    let vm = TestVm::new(
        VmId(format!("gateway-service-{}", key.identity.instance_id)),
        Box::new(host),
        state_sender,
    );
    let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
    registry.register(key, vm, state).expect("register");
    let handler = GatewayServiceHandler::new(
        Resolver {
            target: Ok(GatewayExecutionTarget::Service(budget(
                key,
                Duration::from_millis(25),
            ))),
            delay: Duration::ZERO,
            calls: AtomicUsize::new(0),
        },
        Stateless {
            calls: AtomicUsize::new(0),
        },
        registry,
        owner(),
    )
    .expect("handler");
    let result = handler.invoke(&route, Uuid::new_v4(), request()).await;
    assert!(result.is_err());
    tokio::time::timeout(Duration::from_secs(1), closed_receiver)
        .await
        .expect("service stream closed")
        .expect("close marker");
    drop(handler);
}
