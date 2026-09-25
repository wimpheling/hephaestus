use super::{assertion, rpc_failure_marker};
use identity_domain::BrowserSessionSid;
use uuid::Uuid;

#[test]
fn rpc_failure_marker_keeps_closed_code_and_drops_error_body() {
    let body = br#"{"code":"unavailable","message":"password=do-not-retain"}"#;
    let marker = rpc_failure_marker(
        "/hephaestus.gateway.v1.GatewayService/GetGateway",
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        body,
    );
    assert_eq!(
        marker,
        "HEPH_GCP_RUNTIME error_class=unknown reason_class=unavailable operation=get-gateway status=failed rc=503"
    );
    assert!(!marker.contains("password"));
    assert!(!marker.contains("do-not-retain"));
}

#[test]
fn rpc_failure_marker_maps_status_and_unknown_method_without_body() {
    let marker = rpc_failure_marker(
        "/unknown.Service/Unknown",
        reqwest::StatusCode::NOT_FOUND,
        &[],
    );
    assert_eq!(
        marker,
        "HEPH_GCP_RUNTIME error_class=not-found reason_class=not_found operation=unknown status=failed rc=404"
    );
}

#[test]
fn inspection_assertion_round_trips_the_supplied_session_sid() {
    use http::{HeaderMap, HeaderValue, header::AUTHORIZATION};

    let audience = "/hephaestus.gateway.v1.GatewayService/GetGateway";
    let session_sid = BrowserSessionSid::new();
    let token = assertion(Uuid::new_v4(), audience, session_sid);
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).expect("inspection assertion header"),
    );
    let principal = hephaestus_app::rpc::MediatorAuthenticator::new(
        &hephaestus_app::rpc::mediator_signing_key(
            b"golden-internal-command-token-with-sufficient-entropy",
        ),
    )
    .authenticate(&headers, audience)
    .expect("inspection assertion with the supplied SID authenticates");
    assert_eq!(principal.sid, session_sid);
}
