use super::super::{
    common::GIT_ACTOR_HEADER, parse_child_cookie, require_same_origin, validation::actor_header,
};
use super::support::{test_router, test_router_with_host};
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use forge_domain::RepositoryId;
use git_http::AuthenticatedHumanGitEndpoint;
use release_service::{
    UiNamespace, UiPublicPort,
    ui_browser_host::{UI_CHILD_COOKIE, UiGenerationHost},
};
use std::net::SocketAddr;
use tower::ServiceExt;

#[test]
fn same_origin_requires_exact_generation_origin_for_all_git_requests() {
    let namespace = UiNamespace::parse("ui.example.test").expect("namespace");
    let port = UiPublicPort::https_default();
    let generation = release_domain::UiInstallationGenerationId::from_uuid(
        uuid::Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").expect("uuid"),
    );
    let host = UiGenerationHost::from_generation_id(generation);
    let request = Request::builder()
        .uri("/_heph/git/01234567-89ab-cdef-0123-456789abcdef/info/refs")
        .header(header::HOST, host.authority(&namespace, port))
        .header(
            header::ORIGIN,
            format!("https://{}", host.authority(&namespace, port)),
        )
        .body(Body::empty())
        .expect("request");
    assert!(require_same_origin(&request, host, &namespace, port, true).is_ok());
    let missing = Request::builder()
        .uri("/_heph/git/01234567-89ab-cdef-0123-456789abcdef/info/refs")
        .header(header::HOST, host.authority(&namespace, port))
        .body(Body::empty())
        .expect("request");
    assert!(require_same_origin(&missing, host, &namespace, port, false).is_ok());
    assert!(require_same_origin(&missing, host, &namespace, port, true).is_err());
}

#[test]
fn child_cookie_parser_rejects_duplicates_and_accepts_only_32_bytes() {
    let encoded = URL_SAFE_NO_PAD.encode([7_u8; 32]);
    let request = Request::builder()
        .header(header::COOKIE, format!("{UI_CHILD_COOKIE}={encoded}"))
        .body(Body::empty())
        .expect("request");
    assert!(parse_child_cookie(&request).is_ok());
    let duplicate = Request::builder()
        .header(
            header::COOKIE,
            format!("{UI_CHILD_COOKIE}={encoded}; {UI_CHILD_COOKIE}={encoded}"),
        )
        .body(Body::empty())
        .expect("request");
    assert!(parse_child_cookie(&duplicate).is_err());
}

#[tokio::test]
async fn actual_router_constructs_and_routes_reserved_path() {
    let app = test_router().await;
    let mut request = Request::builder()
        .method("GET")
        .uri(format!(
            "/_heph/git/{}/info/refs?service=unsupported",
            RepositoryId::new()
        ))
        .header(
            header::HOST,
            "g-0123456789abcdef0123456789abcdef.ui.example.test",
        )
        .body(Body::empty())
        .expect("request");
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1234))));
    let response = app.oneshot(request).await.expect("router response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn actual_router_rejects_missing_cookie_and_cross_origin_requests() {
    let generation = release_domain::UiInstallationGenerationId::from_uuid(
        uuid::Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").expect("uuid"),
    );
    let host = UiGenerationHost::from_generation_id(generation);
    let namespace = UiNamespace::parse("ui.example.test").expect("namespace");
    let authority = host.authority(&namespace, UiPublicPort::https_default());
    let app = test_router_with_host(Some(generation)).await;
    let mut missing_cookie = Request::builder()
        .method("GET")
        .uri(format!(
            "/_heph/git/{}/info/refs?service=git-upload-pack",
            generation.as_uuid()
        ))
        .header(header::HOST, &authority)
        .body(Body::empty())
        .expect("missing-cookie request");
    missing_cookie
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1234))));
    assert_eq!(
        app.oneshot(missing_cookie)
            .await
            .expect("missing-cookie response")
            .status(),
        StatusCode::UNAUTHORIZED
    );

    let app = test_router_with_host(Some(generation)).await;
    let encoded = URL_SAFE_NO_PAD.encode([7_u8; 32]);
    let mut cross_origin = Request::builder()
        .method("POST")
        .uri(format!(
            "/_heph/git/{}/git-upload-pack",
            generation.as_uuid()
        ))
        .header(header::HOST, &authority)
        .header(header::ORIGIN, "https://wrong.ui.example.test")
        .header(header::COOKIE, format!("{UI_CHILD_COOKIE}={encoded}"))
        .body(Body::empty())
        .expect("cross-origin request");
    cross_origin
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1234))));
    assert_eq!(
        app.oneshot(cross_origin)
            .await
            .expect("cross-origin response")
            .status(),
        StatusCode::BAD_REQUEST
    );
}

#[test]
fn actor_header_is_only_added_to_discovery_responses() {
    let actor = identity_domain::UserId::new();
    let response = Response::new(Body::empty());
    let response = actor_header(
        response,
        actor,
        AuthenticatedHumanGitEndpoint::CloneInfoRefs,
    );
    assert_eq!(
        response
            .headers()
            .get(GIT_ACTOR_HEADER)
            .unwrap()
            .to_str()
            .unwrap(),
        actor.to_string()
    );
    let response = actor_header(
        Response::new(Body::empty()),
        actor,
        AuthenticatedHumanGitEndpoint::UploadPack,
    );
    assert!(response.headers().get(GIT_ACTOR_HEADER).is_none());
}
