use super::*;

#[tokio::test]
async fn content_router_preserves_child_only_gateway_authority_and_rejects_guest_cookie() {
    let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
    let key = Uuid::new_v4();
    let (artifacts, root) = temp_store(b"unused", key);
    let request_projection = UiGatewayRequestProjection {
        kind: release_service::UiGatewayRequestKind::Api,
        path: release_service::UiBrowserHttpPath::parse("/service/api").expect("path"),
        method: HttpMethod::Post,
    };
    let context = context(generation_id);
    let authority = Arc::new(FakeAuthority {
        calls: AtomicUsize::new(0),
        projection: Mutex::new(Some(UiServingProjection::Gateway {
            context: context.clone(),
            request: request_projection,
        })),
    });
    let gateway = Arc::new(FakeGateway {
        calls: AtomicUsize::new(0),
        captured: Mutex::new(None),
        response: Mutex::new(Some(UiGatewayResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: Bytes::from_static(b"ok"),
        })),
    });
    let (state, host) = host_and_state(
        Arc::clone(&authority),
        artifacts,
        Arc::clone(&gateway),
        generation_id,
    );
    let request = content_request(
        "POST",
        "/service/api?x=%2F%2F",
        &host,
        &[
            ("origin", &format!("https://{host}")),
            ("authorization", "bad"),
        ],
        Body::from("payload"),
    );
    let response = router(Arc::clone(&state))
        .oneshot(request)
        .await
        .expect("gateway");
    assert_eq!(response.status(), StatusCode::OK);
    let captured = gateway.captured.lock().await.take().expect("authority");
    assert_eq!(captured.child_session_id, context.session_id);
    assert_eq!(captured.generation_id, generation_id);
    let response_headers =
        HeaderMap::from_iter([(header::SET_COOKIE, HeaderValue::from_static("guest=x"))]);
    *gateway.response.lock().await = Some(UiGatewayResponse {
        status: StatusCode::OK,
        headers: response_headers,
        body: Bytes::new(),
    });
    let request = content_request(
        "POST",
        "/service/api",
        &host,
        &[("origin", &format!("https://{host}"))],
        Body::empty(),
    );
    let response = router(state)
        .oneshot(request)
        .await
        .expect("guest cookie response");
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn content_router_authorizes_base_redirect_before_artifact_or_guest_access() {
    let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
    let key = Uuid::new_v4();
    let (artifacts, root) = temp_store(b"must not be read", key);
    let context = context(generation_id);
    let authority = Arc::new(FakeAuthority {
        calls: AtomicUsize::new(0),
        projection: Mutex::new(Some(UiServingProjection::Redirect {
            context,
            location: release_service::UiBrowserHttpPath::parse("/docs/index.html")
                .expect("entrypoint"),
        })),
    });
    let gateway = Arc::new(FakeGateway {
        calls: AtomicUsize::new(0),
        captured: Mutex::new(None),
        response: Mutex::new(None),
    });
    let (state, host) = host_and_state(
        Arc::clone(&authority),
        artifacts,
        Arc::clone(&gateway),
        generation_id,
    );
    let request = content_request("GET", "/docs?x=%2F%2F&empty=", &host, &[], Body::empty());
    let response = router(state).oneshot(request).await.expect("redirect");
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        response.headers()[header::LOCATION],
        "/docs/index.html?x=%2F%2F&empty="
    );
    assert_eq!(authority.calls.load(Ordering::SeqCst), 1);
    assert_eq!(gateway.calls.load(Ordering::SeqCst), 0);
    fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn gateway_started_before_deadline_is_audited_as_unknown() {
    let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
    let key = Uuid::new_v4();
    let (artifacts, root) = temp_store(b"unused", key);
    let request_projection = UiGatewayRequestProjection {
        kind: release_service::UiGatewayRequestKind::Api,
        path: release_service::UiBrowserHttpPath::parse("/service/api").expect("path"),
        method: HttpMethod::Post,
    };
    let authority = Arc::new(FakeAuthority {
        calls: AtomicUsize::new(0),
        projection: Mutex::new(Some(UiServingProjection::Gateway {
            context: context(generation_id),
            request: request_projection,
        })),
    });
    let started = Arc::new(AtomicBool::new(false));
    let gateway: Arc<dyn UiGatewayDispatcher> = Arc::new(StartedTimeoutGateway {
        started: Arc::clone(&started),
    });
    let sink = Arc::new(crate::ui_audit::CapturingAuditSink::default());
    let (mut state, host) = host_and_state_with_sink(
        authority,
        artifacts,
        gateway,
        generation_id,
        Arc::clone(&sink) as Arc<dyn release_service::UiRequestAuditSink>,
    );
    Arc::get_mut(&mut state).expect("unique state").deadline = Duration::from_millis(100);
    let request = content_request(
        "POST",
        "/service/api",
        &host,
        &[("origin", &format!("https://{host}"))],
        Body::empty(),
    );
    let response = router(state)
        .oneshot(request)
        .await
        .expect("timeout response");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(started.load(Ordering::SeqCst));
    let events = sink.events.lock().expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].decision(),
        release_service::UiRequestAuditDecision::Undetermined
    );
    assert_eq!(
        events[0].outcome(),
        release_service::UiRequestAuditOutcome::Unknown
    );
    drop(events);
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn gateway_denial_and_guest_forbidden_have_distinct_audit_classes() {
    assert!(matches!(
        gateway_audit_disposition(gateway_edge::UiDispatchDisposition::ProviderDenied),
        GatewayAuditDisposition::Denied(release_service::UiRequestAuditReason::Unauthorized)
    ));
    assert!(matches!(
        gateway_audit_disposition(gateway_edge::UiDispatchDisposition::Admitted {
            outcome: gateway_edge::GatewayInvocationOutcome::Completed,
            completion_persisted: true,
        }),
        GatewayAuditDisposition::Succeeded
    ));
}

#[test]
fn gateway_timeout_and_completion_store_failure_are_unknown() {
    assert!(matches!(
        gateway_audit_disposition(gateway_edge::UiDispatchDisposition::Admitted {
            outcome: gateway_edge::GatewayInvocationOutcome::TimedOut,
            completion_persisted: true,
        }),
        GatewayAuditDisposition::Unknown
    ));
    assert!(matches!(
        gateway_audit_disposition(gateway_edge::UiDispatchDisposition::Admitted {
            outcome: gateway_edge::GatewayInvocationOutcome::Completed,
            completion_persisted: false,
        }),
        GatewayAuditDisposition::Unknown
    ));
}
