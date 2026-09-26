use super::*;
use crate::GatewayLimits;
use crate::{
    Exposure, GatewayResponse, GatewayRouteBinding, UiGatewayAdmission, UiGatewayAdmissionError,
    UiGatewayAuthority, UiGatewayRequestKind,
};
use bytes::Bytes;
use http::{HeaderMap, HeaderValue, Method, StatusCode};
use std::{collections::BTreeSet, time::Duration};
use uuid::Uuid;

fn authority(method: Method, path: &str) -> UiGatewayAuthority {
    UiGatewayAuthority {
        child_session_id: Uuid::new_v4(),
        actor_id: Uuid::new_v4(),
        organization_id: Uuid::new_v4(),
        installation_id: Uuid::new_v4(),
        generation_id: Uuid::new_v4(),
        canonical_request_path: path.to_owned(),
        request_kind: UiGatewayRequestKind::Managed,
        method,
    }
}

fn route(exposure: Exposure) -> GatewayRouteBinding {
    GatewayRouteBinding {
        route_id: Uuid::new_v4(),
        gateway_revision_id: Uuid::new_v4(),
        exposure,
        path_prefix: "ui/release".to_owned(),
        methods: BTreeSet::from([Method::GET]),
        limits: GatewayLimits {
            max_request_body_bytes: 1024,
            max_response_body_bytes: 1024,
            max_request_headers: 32,
            max_response_headers: 32,
            max_path_and_query_bytes: 1024,
            execution_timeout: Duration::from_secs(1),
        },
    }
}

#[test]
fn managed_and_api_paths_have_distinct_canonical_grammars() {
    let managed = authority(Method::GET, "ui/release/index.html");
    assert!(
        managed
            .validate_for(&Method::GET, "ui/release/index.html?next=%2F//opaque")
            .is_ok()
    );
    assert!(
        managed
            .validate_for(&Method::GET, "/ui/release/index.html")
            .is_err()
    );
    assert!(
        managed
            .validate_for(&Method::GET, "ui/release/../secret")
            .is_err()
    );

    let api = UiGatewayAuthority {
        request_kind: UiGatewayRequestKind::Api,
        canonical_request_path: "/api/v1/items".to_owned(),
        ..managed
    };
    assert!(
        api.validate_for(&Method::GET, "/api/v1/items?next=%2F//opaque")
            .is_ok()
    );
    assert!(api.validate_for(&Method::GET, "api/v1/items").is_err());
    assert!(
        api.validate_for(&Method::GET, "/api/v1/%2e%2e/secret")
            .is_err()
    );
}

#[test]
fn admission_preserves_opaque_query_and_rejects_route_mismatch() {
    let authority = authority(Method::GET, "ui/release/index.html");
    let admission = UiGatewayAdmission {
        route: route(Exposure::HephAuthenticated),
        gateway_path_and_query: "/gateway/ui/release/index.html?next=%2F//opaque".to_owned(),
    };
    assert!(
        admission
            .validate_for(
                &authority,
                &Method::GET,
                "ui/release/index.html?next=%2F//opaque",
            )
            .is_ok()
    );
    let mismatch = UiGatewayAdmission {
        gateway_path_and_query: "/gateway/another-release/index.html".to_owned(),
        ..admission
    };
    assert!(
        mismatch
            .validate_for(&authority, &Method::GET, "ui/release/index.html")
            .is_err()
    );
}

#[test]
fn public_routes_and_invalid_authority_are_rejected() {
    let authority = authority(Method::GET, "ui/release/index.html");
    let public = UiGatewayAdmission {
        route: route(Exposure::Public),
        gateway_path_and_query: "/gateway/ui/release/index.html".to_owned(),
    };
    assert!(
        public
            .validate_for(&authority, &Method::GET, "ui/release/index.html")
            .is_err()
    );
    let mut nil = authority;
    nil.child_session_id = Uuid::nil();
    assert!(
        nil.validate_for(&Method::GET, "ui/release/index.html")
            .is_err()
    );
}

#[test]
fn guest_headers_and_set_cookie_are_rejected() {
    let mut headers = HeaderMap::new();
    for name in [
        "authorization",
        "cookie",
        "host",
        "proxy-authorization",
        "x-api-key",
        "x-auth-token",
        "x-access-token",
        "forwarded",
        "x-forwarded-for",
        "x-forwarded-port",
        "x-forwarded-prefix",
        "x-forwarded-server",
    ] {
        headers.insert(name, HeaderValue::from_static("secret"));
    }
    strip_ui_guest_headers(&mut headers);
    assert!(headers.is_empty());

    let mut response = GatewayResponse {
        status: StatusCode::OK,
        headers: HeaderMap::new(),
        body: Bytes::new(),
        mailbox_publication: None,
    };
    response
        .headers
        .insert("set-cookie", HeaderValue::from_static("sid=secret"));
    assert!(reject_ui_set_cookie(&response).is_err());
}

#[test]
fn admission_errors_have_safe_statuses() {
    assert_eq!(
        admission_failure_response(UiGatewayAdmissionError::Denied).status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        admission_failure_response(UiGatewayAdmissionError::NotFound).status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        admission_failure_response(UiGatewayAdmissionError::Unavailable).status,
        StatusCode::SERVICE_UNAVAILABLE
    );
}
