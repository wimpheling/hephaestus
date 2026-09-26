use super::*;

#[test]
fn canonical_request_keeps_query_opaque() {
    let request = Request::builder()
        .method("GET")
        .uri("/docs/app.js?x=%2F%2F&heph_theme=dark")
        .header(
            header::HOST,
            "g-0123456789abcdef0123456789abcdef.ui.app.example",
        )
        .body(Body::empty())
        .expect("request");
    let parsed = UiHttpRequest::parse(&request).expect("canonical request");
    assert_eq!(parsed.path.as_str(), "/docs/app.js");
    assert_eq!(parsed.query.as_deref(), Some("x=%2F%2F&heph_theme=dark"));
}

#[test]
fn canonical_alias_redirect_is_temporary_and_preserves_opaque_query() {
    let location =
        release_service::UiBrowserHttpPath::parse("/docs/index.html").expect("entrypoint");
    let csp = platform_csp_value("https://platform.example").expect("CSP");
    let response = redirect_response(&location, Some(""), &csp).expect("redirect");
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(response.headers()[header::LOCATION], "/docs/index.html?");
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
}

#[test]
fn malformed_path_and_duplicate_host_fail_closed() {
    for uri in [
        "/docs/../secret",
        "/docs/%2e%2e/secret",
        "/docs//x",
        "/docs/",
    ] {
        let request = Request::builder()
            .method("GET")
            .uri(uri)
            .header(
                header::HOST,
                "g-0123456789abcdef0123456789abcdef.ui.app.example",
            )
            .body(Body::empty())
            .expect("request");
        assert!(UiHttpRequest::parse(&request).is_err(), "{uri}");
    }
    let request = Request::builder()
        .method("GET")
        .uri("/docs/index.html")
        .header(header::HOST, "one.example")
        .header(header::HOST, "two.example")
        .body(Body::empty())
        .expect("request");
    assert!(UiHttpRequest::parse(&request).is_err());
}

#[test]
fn cookie_requires_one_exact_unpadded_secret() {
    let value = URL_SAFE_NO_PAD.encode([7_u8; 32]);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        HeaderValue::from_str(&format!("{UI_CHILD_COOKIE}={value}")).expect("cookie header"),
    );
    assert!(parse_child_cookie(&headers).is_ok());
    headers.insert(
        header::COOKIE,
        HeaderValue::from_str(&format!(
            "{UI_CHILD_COOKIE}={value}; {UI_CHILD_COOKIE}={value}"
        ))
        .expect("duplicate cookie header"),
    );
    assert!(parse_child_cookie(&headers).is_err());
}

#[test]
fn ranges_are_single_and_bounded() {
    assert_eq!(parse_single_range("bytes=2-4", 8), Ok((2, 4)));
    assert_eq!(parse_single_range("bytes=5-", 8), Ok((5, 7)));
    assert_eq!(parse_single_range("bytes=-2", 8), Ok((6, 7)));
    assert!(parse_single_range("bytes=2-4,6-7", 8).is_err());
    assert!(parse_single_range("bytes=9-10", 8).is_err());
}

#[test]
fn response_policy_forces_nosniff_on_guest_success() {
    let response = gateway_response(
        UiGatewayResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: Bytes::from_static(b"ok"),
        },
        &platform_csp_value("https://platform.example").expect("CSP"),
    )
    .expect("response");
    assert_eq!(
        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
}

#[test]
fn response_policy_rejects_guest_set_cookie() {
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, HeaderValue::from_static("guest=x"));
    let response = UiGatewayResponse {
        status: StatusCode::OK,
        headers,
        body: Bytes::new(),
    };
    assert!(matches!(
        gateway_response(
            response,
            &platform_csp_value("https://platform.example").expect("CSP"),
        ),
        Err(UiContentError::GuestSetCookie)
    ));
}

#[test]
fn unsafe_requests_require_exact_generation_origin() {
    let namespace = UiNamespace::parse("ui.app.example").expect("namespace");
    let port = UiPublicPort::https_default();
    let host = UiGenerationHost::from_generation_id(
        release_domain::UiInstallationGenerationId::from_uuid(
            uuid::Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").expect("UUID"),
        ),
    );
    let expected = format!("https://{}", host.authority(&namespace, port));
    let request = Request::builder()
        .method("POST")
        .uri("/service/api")
        .header(header::HOST, host.authority(&namespace, port))
        .header(header::ORIGIN, expected)
        .header("sec-fetch-site", "same-origin")
        .body(Body::empty())
        .expect("request");
    assert!(require_unsafe_origin(&request, &host, HttpMethod::Post, &namespace, port).is_ok());
    let foreign = Request::builder()
        .method("POST")
        .uri("/service/api")
        .header(header::HOST, host.authority(&namespace, port))
        .header(header::ORIGIN, "null")
        .body(Body::empty())
        .expect("foreign request");
    assert!(require_unsafe_origin(&foreign, &host, HttpMethod::Post, &namespace, port).is_err());
}

#[test]
fn platform_csp_cannot_be_overridden_by_guest_headers() {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src *"),
    );
    let sanitized = sanitize_guest_response_headers(headers);
    assert!(sanitized.get(header::CONTENT_SECURITY_POLICY).is_none());
    let mut final_headers = HeaderMap::new();
    let csp = platform_csp_value("https://platform.example").expect("CSP");
    apply_platform_csp(&mut final_headers, &csp);
    assert_eq!(
        final_headers
            .get(header::CONTENT_SECURITY_POLICY)
            .and_then(|value| value.to_str().ok()),
        Some(
            "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; font-src 'self'; worker-src 'none'; object-src 'none'; frame-src 'none'; frame-ancestors https://platform.example; connect-src 'self'; form-action 'none'; base-uri 'none'"
        )
    );
}

#[test]
fn platform_origin_validation_requires_same_site_exact_https_origin() {
    let port = UiPublicPort::https_default();
    let namespace = UiNamespace::parse("ui.app.example").expect("namespace");
    assert!(UiOriginConfig::new(namespace.clone(), port, "https://app.example").is_ok());
    assert!(UiOriginConfig::new(namespace.clone(), port, "https://other.example").is_err());
    assert!(UiOriginConfig::new(namespace.clone(), port, "https://app.example/path").is_err());
    assert!(UiOriginConfig::new(namespace, port, "https://app.example:0443").is_err());
}

#[test]
fn etag_is_lowercase_hex() {
    let mut bytes = [0_u8; 32];
    bytes[1] = 0xab;
    bytes[2] = 0xff;
    assert_eq!(hex_bytes(&bytes), format!("00abff{}", "00".repeat(29)));
}
