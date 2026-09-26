use super::super::{UiBootstrapConfig, UiBootstrapState, router};
use super::support::{
    FakeHostResolver, FakeSessions, bootstrap_state, bootstrap_state_with_rejection,
};
use axum::{
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use release_domain::UiInstallationGenerationId;
use release_service::ui_browser_host::{
    UI_BOOTSTRAP_PATH, UiGenerationHost, UiNamespace, UiPublicPort,
};
use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
async fn bootstrap_router_appends_correlation_before_exchange_denial() {
    let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
    let host_calls = Arc::new(AtomicUsize::new(0));
    let exchange_calls = Arc::new(AtomicUsize::new(0));
    let sink = Arc::new(crate::ui_audit::CapturingAuditSink::default());
    let namespace = UiNamespace::parse("ui.app.example").expect("namespace");
    let port = UiPublicPort::https_default();
    let host = UiGenerationHost::from_generation_id(generation_id);
    let authority = host.authority(&namespace, port);
    let state = Arc::new(UiBootstrapState::new(
        Arc::new(FakeHostResolver {
            generation_id,
            calls: host_calls,
        }),
        Arc::new(FakeSessions {
            exchanges: exchange_calls,
            reject: true,
        }),
        UiBootstrapConfig::new(namespace, port, "https://app.example").expect("config"),
        Arc::clone(&sink) as Arc<dyn release_service::UiRequestAuditSink>,
    ));
    let mut request = Request::builder()
        .method("POST")
        .uri(UI_BOOTSTRAP_PATH)
        .header(header::HOST, &authority)
        .header(header::ORIGIN, format!("https://{authority}"))
        .body(Body::from(URL_SAFE_NO_PAD.encode([7_u8; 32])))
        .expect("request");
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 4321))));
    let response = router(state).oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let events = sink.events.lock().expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].surface(),
        release_service::UiRequestAuditSurface::Bootstrap
    );
    assert_eq!(
        events[0].decision(),
        release_service::UiRequestAuditDecision::Denied
    );
    assert!(events[0].context().actor_id().is_none());
    drop(events);
}

#[tokio::test]
async fn bootstrap_router_exchanges_valid_fragment_and_returns_safe_cookie_json() {
    let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
    let host_calls = Arc::new(AtomicUsize::new(0));
    let exchange_calls = Arc::new(AtomicUsize::new(0));
    let (state, authority) = bootstrap_state(
        generation_id,
        Arc::clone(&host_calls),
        Arc::clone(&exchange_calls),
    );
    let fragment = URL_SAFE_NO_PAD.encode([7_u8; 32]);
    let mut request = Request::builder()
        .method("POST")
        .uri(UI_BOOTSTRAP_PATH)
        .header(header::HOST, &authority)
        .header(header::ORIGIN, format!("https://{authority}"))
        .body(Body::from(fragment.clone()))
        .expect("request");
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 4321))));
    let response = router(state).oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("child cookie");
    assert!(cookie.starts_with("__Host-hephaestus_ui="));
    assert!(cookie.contains("Secure"));
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    assert!(!cookie.contains("Domain="));
    let body = to_bytes(response.into_body(), 4096).await.expect("body");
    let body = String::from_utf8(body.to_vec()).expect("JSON");
    assert!(body.contains("/schema-ui"));
    assert!(!body.contains(&fragment));
    assert_eq!(host_calls.load(Ordering::SeqCst), 1);
    assert_eq!(exchange_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn bootstrap_router_rejects_duplicate_host_origin_and_non_loopback_before_exchange() {
    let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
    let host_calls = Arc::new(AtomicUsize::new(0));
    let exchange_calls = Arc::new(AtomicUsize::new(0));
    let (state, authority) = bootstrap_state(
        generation_id,
        Arc::clone(&host_calls),
        Arc::clone(&exchange_calls),
    );
    let fragment = URL_SAFE_NO_PAD.encode([7_u8; 32]);
    let mut duplicate = Request::builder()
        .method("POST")
        .uri(UI_BOOTSTRAP_PATH)
        .header(header::HOST, &authority)
        .header(header::HOST, &authority)
        .header(header::ORIGIN, format!("https://{authority}"))
        .body(Body::from(fragment.clone()))
        .expect("request");
    duplicate
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 4321))));
    let response = router(Arc::clone(&state))
        .oneshot(duplicate)
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(exchange_calls.load(Ordering::SeqCst), 0);

    let mut absolute = Request::builder()
        .method("POST")
        .uri(format!("https://{authority}{UI_BOOTSTRAP_PATH}"))
        .header(header::HOST, &authority)
        .header(header::ORIGIN, format!("https://{authority}"))
        .body(Body::from(fragment.clone()))
        .expect("absolute-form request");
    absolute
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 4321))));
    let response = router(Arc::clone(&state))
        .oneshot(absolute)
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let mut foreign = Request::builder()
        .method("POST")
        .uri(UI_BOOTSTRAP_PATH)
        .header(header::HOST, &authority)
        .header(header::ORIGIN, "https://other.example")
        .body(Body::from(fragment))
        .expect("request");
    foreign
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([192, 0, 2, 1], 4321))));
    let response = router(state).oneshot(foreign).await.expect("response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(exchange_calls.load(Ordering::SeqCst), 0);
    assert_eq!(host_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn bootstrap_router_rejects_expired_exchange_without_cookie_or_safe_route() {
    let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
    let host_calls = Arc::new(AtomicUsize::new(0));
    let exchange_calls = Arc::new(AtomicUsize::new(0));
    let (state, authority) = bootstrap_state_with_rejection(
        generation_id,
        host_calls,
        Arc::clone(&exchange_calls),
        true,
    );
    let mut request = Request::builder()
        .method("POST")
        .uri(UI_BOOTSTRAP_PATH)
        .header(header::HOST, &authority)
        .header(header::ORIGIN, format!("https://{authority}"))
        .body(Body::from(URL_SAFE_NO_PAD.encode([8_u8; 32])))
        .expect("request");
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 4321))));
    let response = router(state).oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(response.headers().get(header::SET_COOKIE).is_none());
    assert_eq!(exchange_calls.load(Ordering::SeqCst), 1);
}
