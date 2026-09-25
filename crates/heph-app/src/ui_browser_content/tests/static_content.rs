use super::*;

#[tokio::test]
async fn content_router_audits_verified_static_success_before_bytes() {
    let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
    let key = Uuid::new_v4();
    let (artifacts, root) = temp_store(b"audited", key);
    let context = context(generation_id);
    let authority = Arc::new(FakeAuthority {
        calls: AtomicUsize::new(0),
        projection: Mutex::new(Some(UiServingProjection::Static {
            context,
            artifact: static_artifact(b"audited", key),
        })),
    });
    let gateway = Arc::new(FakeGateway {
        calls: AtomicUsize::new(0),
        captured: Mutex::new(None),
        response: Mutex::new(None),
    });
    let sink = Arc::new(crate::ui_audit::CapturingAuditSink::default());
    let (state, host) = host_and_state_with_sink(
        authority,
        artifacts,
        gateway,
        generation_id,
        Arc::clone(&sink) as Arc<dyn release_service::UiRequestAuditSink>,
    );
    let request = content_request("GET", "/docs/index.html", &host, &[], Body::empty());
    let response = router(state).oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), 128).await.expect("body"),
        "audited"
    );
    let events = sink.events.lock().expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].surface(),
        release_service::UiRequestAuditSurface::Static
    );
    assert_eq!(
        events[0].decision(),
        release_service::UiRequestAuditDecision::Allowed
    );
    assert_eq!(
        events[0].outcome(),
        release_service::UiRequestAuditOutcome::Succeeded
    );
    assert!(events[0].context().child_session_id().is_some());
    drop(events);
    fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn content_router_keeps_denial_when_audit_sink_is_unavailable() {
    let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
    let key = Uuid::new_v4();
    let (artifacts, root) = temp_store(b"unused", key);
    let authority = Arc::new(FakeAuthority {
        calls: AtomicUsize::new(0),
        projection: Mutex::new(None),
    });
    let gateway = Arc::new(FakeGateway {
        calls: AtomicUsize::new(0),
        captured: Mutex::new(None),
        response: Mutex::new(None),
    });
    let sink = Arc::new(crate::ui_audit::CapturingAuditSink::default());
    sink.fail.store(true, Ordering::Relaxed);
    let (state, host) = host_and_state_with_sink(
        authority,
        artifacts,
        gateway,
        generation_id,
        Arc::clone(&sink) as Arc<dyn release_service::UiRequestAuditSink>,
    );
    let request = content_request("GET", "/docs/index.html", &host, &[], Body::empty());
    let response = router(state).oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(sink.events.lock().expect("events").is_empty());
    fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn content_router_denies_cross_generation_unsafe_origin_before_authority() {
    let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
    let key = Uuid::new_v4();
    let (artifacts, root) = temp_store(b"content", key);
    let authority = Arc::new(FakeAuthority {
        calls: AtomicUsize::new(0),
        projection: Mutex::new(None),
    });
    let gateway = Arc::new(FakeGateway {
        calls: AtomicUsize::new(0),
        captured: Mutex::new(None),
        response: Mutex::new(None),
    });
    let (state, host) = host_and_state(authority.clone(), artifacts, gateway, generation_id);
    let request = content_request(
        "POST",
        "/service/api",
        &host,
        &[("origin", "https://other.ui.app.example")],
        Body::from("body"),
    );
    let response = router(state).oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(authority.calls.load(Ordering::SeqCst), 0);
    fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Covers the complete static response matrix.
async fn content_router_serves_verified_static_etag_range_head_and_tamper_failure() {
    let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
    let key = Uuid::new_v4();
    let bytes = b"verified-content";
    let (artifacts, root) = temp_store(bytes, key);
    let artifact = static_artifact(bytes, key);
    let authority = Arc::new(FakeAuthority {
        calls: AtomicUsize::new(0),
        projection: Mutex::new(Some(UiServingProjection::Static {
            context: context(generation_id),
            artifact: artifact.clone(),
        })),
    });
    let gateway = Arc::new(FakeGateway {
        calls: AtomicUsize::new(0),
        captured: Mutex::new(None),
        response: Mutex::new(None),
    });
    let (mut state, host) =
        host_and_state(Arc::clone(&authority), artifacts, gateway, generation_id);
    let etag = format!("\"{}\"", hex_bytes(artifact.content_hash.as_bytes()));
    let request = content_request(
        "GET",
        "/schema-ui/index.txt",
        &host,
        &[("if-none-match", &etag)],
        Body::empty(),
    );
    let response = router(Arc::clone(&state))
        .oneshot(request)
        .await
        .expect("304");
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let duplicate_cookie = format!(
        "{UI_CHILD_COOKIE}={}; {UI_CHILD_COOKIE}={}",
        URL_SAFE_NO_PAD.encode([7_u8; 32]),
        URL_SAFE_NO_PAD.encode([8_u8; 32])
    );
    let request = content_request(
        "GET",
        "/schema-ui/index.txt",
        &host,
        &[("cookie", &duplicate_cookie)],
        Body::empty(),
    );
    let response = router(Arc::clone(&state))
        .oneshot(request)
        .await
        .expect("duplicate cookie response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let request = content_request(
        "GET",
        "/schema-ui/index.txt",
        &host,
        &[("range", "bytes=1-3"), ("if-range", &etag)],
        Body::empty(),
    );
    let response = router(Arc::clone(&state))
        .oneshot(request)
        .await
        .expect("206");
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        to_bytes(response.into_body(), 32)
            .await
            .expect("range")
            .as_ref(),
        b"eri"
    );
    let request = content_request("HEAD", "/schema-ui/index.txt", &host, &[], Body::empty());
    let response = router(Arc::clone(&state))
        .oneshot(request)
        .await
        .expect("HEAD");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        to_bytes(response.into_body(), 32)
            .await
            .expect("head")
            .is_empty()
    );
    *authority.projection.lock().await = None;
    let request = content_request(
        "GET",
        "/schema-ui/index.txt",
        &host,
        &[("if-none-match", &etag), ("range", "bytes=0-1")],
        Body::empty(),
    );
    let response = router(Arc::clone(&state))
        .oneshot(request)
        .await
        .expect("denial");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    *authority.projection.lock().await = Some(UiServingProjection::Static {
        context: context(generation_id),
        artifact: artifact.clone(),
    });
    Arc::get_mut(&mut state)
        .expect("exclusive state")
        .max_static_bytes = 2;
    let request = content_request("GET", "/schema-ui/index.txt", &host, &[], Body::empty());
    let response = router(Arc::clone(&state))
        .oneshot(request)
        .await
        .expect("size response");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    Arc::get_mut(&mut state)
        .expect("exclusive state")
        .max_static_bytes = DEFAULT_MAX_STATIC_BYTES;
    fs::write(root.join(key.simple().to_string()), b"tampered").expect("tamper");
    let request = content_request("GET", "/schema-ui/index.txt", &host, &[], Body::empty());
    let response = router(state)
        .oneshot(request)
        .await
        .expect("tamper response");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    fs::remove_dir_all(root).expect("cleanup");
}
