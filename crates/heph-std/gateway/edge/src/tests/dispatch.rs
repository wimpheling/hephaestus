use super::*;

#[tokio::test]
async fn reconciliation_is_deterministic_and_idempotent() {
    let admin = Arc::new(Admin(Mutex::new(Vec::new())));
    let provider = LocalCaddyGatewayProvider::new(
        Arc::clone(&admin),
        Arc::new(GatewayDispatcher::new(
            Resolver(route()),
            Handler {
                calls: AtomicUsize::new(0),
                delay: Duration::ZERO,
            },
            Recorder,
        )),
    )
    .with_dispatcher_upstream(String::from("127.0.0.1:19090"))
    .with_configuration_template(caddy_template());
    let desired = GatewayDesiredConfiguration {
        revision: GatewayConfigRevision::new(),
        routes: vec![route()],
    };
    provider
        .reconcile(&desired)
        .await
        .expect("first reconciliation");
    provider
        .reconcile(&desired)
        .await
        .expect("duplicate reconciliation");
    let loaded = admin.0.lock().expect("lock");
    assert_eq!(loaded.len(), 1);
    assert!(
        String::from_utf8_lossy(&loaded[0]).contains("127.0.0.1:19090"),
        "the derived Caddy configuration targets only the configured private dispatcher"
    );
    drop(loaded);
}

#[tokio::test]
async fn failed_reconciliation_does_not_advance_observed_revision() {
    let provider = LocalCaddyGatewayProvider::new(
        FailingAdmin,
        GatewayDispatcher::new(
            Resolver(route()),
            Handler {
                calls: AtomicUsize::new(0),
                delay: Duration::ZERO,
            },
            Recorder,
        ),
    )
    .with_configuration_template(caddy_template());
    let desired = GatewayDesiredConfiguration {
        revision: GatewayConfigRevision::new(),
        routes: vec![route()],
    };
    assert!(provider.reconcile(&desired).await.is_err());
    assert_eq!(provider.observed_revision().await, None);
}

#[tokio::test]
async fn recovery_reapplies_an_unchanged_authoritative_revision() {
    let admin = Arc::new(Admin(Mutex::new(Vec::new())));
    let provider = LocalCaddyGatewayProvider::new(
        Arc::clone(&admin),
        GatewayDispatcher::new(
            Resolver(route()),
            Handler {
                calls: AtomicUsize::new(0),
                delay: Duration::ZERO,
            },
            Recorder,
        ),
    )
    .with_configuration_template(caddy_template());
    let desired = GatewayDesiredConfiguration {
        revision: GatewayConfigRevision::new(),
        routes: vec![route()],
    };
    provider
        .reconcile(&desired)
        .await
        .expect("initial Caddy configuration");
    provider
        .recover(&desired)
        .await
        .expect("Caddy restart recovery");
    assert_eq!(admin.0.lock().expect("lock").len(), 2);
}
#[tokio::test]
async fn dispatcher_times_out_and_never_relays_unbounded_handler() {
    let dispatcher = GatewayDispatcher::new(
        Resolver(route()),
        Handler {
            calls: AtomicUsize::new(0),
            delay: Duration::from_secs(1),
        },
        Recorder,
    );
    let response = dispatcher.dispatch(request("/gateway/echo")).await;
    assert_eq!(response.response.status, StatusCode::GATEWAY_TIMEOUT);
}

#[tokio::test]
async fn inbound_secret_is_constant_time_checked_and_rewritten_before_vm_delivery() {
    let capture = Arc::new(HeaderCapture(Mutex::new(None)));
    let dispatcher = GatewayDispatcher::new(Resolver(route()), Arc::clone(&capture), Recorder)
        .with_inbound_secret_resolver(Arc::new(InboundRules));
    let mut valid = request("/gateway/echo");
    valid.headers.insert(
        "x-hook-secret",
        HeaderValue::from_static("gateway-secret-sentinel"),
    );
    assert_eq!(
        dispatcher.dispatch(valid).await.response.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        capture.0.lock().expect("capture").as_ref(),
        Some(&HeaderValue::from_static(
            "heph-placeholder:v1:gateway-proof"
        ))
    );
    for values in [
        vec![],
        vec!["wrong-secret"],
        vec!["gateway-secret-sentinel", "gateway-secret-sentinel"],
    ] {
        *capture.0.lock().expect("clear capture") = None;
        let mut invalid = request("/gateway/echo");
        for value in values {
            invalid
                .headers
                .append("x-hook-secret", HeaderValue::from_static(value));
        }
        let response = dispatcher.dispatch(invalid).await.response;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
        assert!(response.body.is_empty());
        assert!(
            capture.0.lock().expect("capture").is_none(),
            "rejected credentials never reach the guest"
        );
    }
}
#[tokio::test]
async fn dispatcher_rejects_forwarded_header_before_launch() {
    let handler = Handler {
        calls: AtomicUsize::new(0),
        delay: Duration::ZERO,
    };
    let dispatcher = GatewayDispatcher::new(Resolver(route()), handler, Recorder)
        .with_mailbox_publisher(Arc::new(AcceptMailbox));
    let mut inbound = request("/gateway/echo");
    inbound
        .headers
        .insert("x-forwarded-for", HeaderValue::from_static("attacker"));
    assert_eq!(
        dispatcher.dispatch(inbound).await.response.status,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn shared_caddy_seam_relays_bounded_status_headers_and_body() {
    let provider = LocalCaddyGatewayProvider::new(
        Admin(Mutex::new(Vec::new())),
        GatewayDispatcher::new(Resolver(route()), EchoHandler, Recorder),
    );
    let response = provider
        .forward(request("/gateway/echo?source=caddy"))
        .await;
    assert_ne!(response.invocation_id, Uuid::nil());
    assert_eq!(response.response.status, StatusCode::ACCEPTED);
    assert_eq!(response.response.body, Bytes::from_static(b"ok"));
    assert_eq!(
        response.response.headers.get("x-gateway-request-id"),
        Some(&HeaderValue::from_static("trusted"))
    );
}

#[tokio::test]
async fn dispatcher_does_not_treat_a_prefix_collision_as_its_route() {
    let handler = Handler {
        calls: AtomicUsize::new(0),
        delay: Duration::ZERO,
    };
    let dispatcher = GatewayDispatcher::new(Resolver(route()), handler, Recorder);
    assert_eq!(
        dispatcher
            .dispatch(request("/gateway/echo-unrelated"))
            .await
            .response
            .status,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn dispatcher_bridges_to_a_private_vm_http_handler_without_networking() {
    let handler = PrivateHttpVmGatewayHandler::new(FakeGatewayLauncher {
        provider: FakeProvider::new().with_private_http_responder(Arc::new(PrivateEcho)),
    });
    let dispatcher = GatewayDispatcher::new(Resolver(route()), handler, Recorder)
        .with_mailbox_publisher(Arc::new(AcceptMailbox));
    let response = dispatcher.dispatch(request("/gateway/echo")).await;
    assert_eq!(response.response.status, StatusCode::CREATED);
    assert_eq!(response.response.body, Bytes::from_static(b"ok"));
    assert!(response.response.mailbox_publication.is_none());
}

#[tokio::test]
async fn dispatcher_fails_closed_when_publication_cannot_be_durably_settled() {
    let publication = Arc::new(CountingMailbox(AtomicUsize::new(0)));
    let recorder = CompletionFailureRecorder {
        completions: AtomicUsize::new(0),
    };
    let dispatcher = GatewayDispatcher::new(
        Resolver(route()),
        PrivateHttpVmGatewayHandler::new(FakeGatewayLauncher {
            provider: FakeProvider::new().with_private_http_responder(Arc::new(PrivateEcho)),
        }),
        recorder,
    )
    .with_mailbox_publisher(publication.clone());

    let response = dispatcher.dispatch(request("/gateway/echo")).await;

    assert_eq!(publication.0.load(Ordering::SeqCst), 1);
    assert_eq!(response.response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.response.body.is_empty());
    assert_ne!(response.invocation_id, Uuid::nil());
    assert_eq!(dispatcher.recorder.completions.load(Ordering::SeqCst), 1);
}

#[test]
fn mailbox_publication_allows_an_empty_opaque_body() {
    assert!(
        validate_mailbox_publication(&PrivateMailboxPublication {
            slot: String::from("recipe-events"),
            method: String::from("POST"),
            route: String::from("/recipe"),
            headers: Vec::new(),
            content_type: None,
            trace_context: None,
            body: Bytes::new(),
            deduplication_key: String::from("empty-body"),
        })
        .is_ok()
    );
}
