use super::assertion;
use identity_domain::BrowserSessionSid;
use serde_json::Value;
use uuid::Uuid;

pub(crate) async fn get_json(
    client: &reqwest::Client,
    running: &hephaestus_app::RunningHephaestus,
    owner: Uuid,
    audience: &str,
    request: Value,
    owner_browser_session: BrowserSessionSid,
) -> Value {
    let response = client
        .post(format!("http://{}{audience}", running.http_addr()))
        .bearer_auth(assertion(owner, audience, owner_browser_session))
        .json(&request)
        .send()
        .await
        .expect("authorized provenance query");
    let status = response.status();
    if status != reqwest::StatusCode::OK {
        // Preserve only bounded transport facts before the assertion panic.
        // The Connect body is parsed and discarded so an application error
        // cannot put credentials or request data into the workload stream.
        let body = response.bytes().await.unwrap_or_default();
        eprintln!("{}", rpc_failure_marker(audience, status, &body));
        assert_eq!(status, reqwest::StatusCode::OK, "provenance RPC {audience}");
        unreachable!("the non-OK provenance assertion must panic");
    }
    assert_eq!(status, reqwest::StatusCode::OK, "provenance RPC {audience}");
    response.json().await.expect("provenance response")
}

pub(crate) fn rpc_failure_marker(
    audience: &str,
    status: reqwest::StatusCode,
    body: &[u8],
) -> String {
    let method = match audience {
        "/hephaestus.gateway.v1.GatewayService/GetGateway" => "get-gateway",
        "/hephaestus.gateway.v1.GatewayService/ListMailboxPublications" => {
            "list-mailbox-publications"
        }
        "/hephaestus.instance.v1.AgentInstanceService/GetInstance" => "get-instance",
        "/hephaestus.run.v1.RunService/GetRun" => "get-run",
        "/hephaestus.run.v1.RunService/GetRunProvenance" => "get-run-provenance",
        _ => "unknown",
    };
    let code = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("code")
                .and_then(Value::as_str)
                .and_then(valid_connect_code)
        })
        .or_else(|| status_connect_code(status))
        .unwrap_or("unknown");
    let error_class = match code {
        "permission_denied" => "permission-denied",
        "not_found" => "not-found",
        "invalid_argument" => "invalid-argument",
        "deadline_exceeded" => "timeout",
        _ => "unknown",
    };
    format!(
        "HEPH_GCP_RUNTIME error_class={error_class} reason_class={code} operation={method} status=failed rc={}",
        status.as_u16()
    )
}

pub(crate) fn valid_connect_code(value: &str) -> Option<&'static str> {
    match value {
        "unauthenticated" => Some("unauthenticated"),
        "invalid_argument" => Some("invalid_argument"),
        "deadline_exceeded" => Some("deadline_exceeded"),
        "not_found" => Some("not_found"),
        "already_exists" => Some("already_exists"),
        "permission_denied" => Some("permission_denied"),
        "resource_exhausted" => Some("resource_exhausted"),
        "failed_precondition" => Some("failed_precondition"),
        "aborted" => Some("aborted"),
        "out_of_range" => Some("out_of_range"),
        "unimplemented" => Some("unimplemented"),
        "internal" => Some("internal"),
        "unavailable" => Some("unavailable"),
        "data_loss" => Some("data_loss"),
        "canceled" => Some("canceled"),
        _ => None,
    }
}

pub(crate) const fn status_connect_code(status: reqwest::StatusCode) -> Option<&'static str> {
    match status {
        reqwest::StatusCode::UNAUTHORIZED => Some("unauthenticated"),
        reqwest::StatusCode::FORBIDDEN => Some("permission_denied"),
        reqwest::StatusCode::BAD_REQUEST => Some("invalid_argument"),
        reqwest::StatusCode::NOT_FOUND => Some("not_found"),
        reqwest::StatusCode::REQUEST_TIMEOUT | reqwest::StatusCode::GATEWAY_TIMEOUT => {
            Some("deadline_exceeded")
        }
        reqwest::StatusCode::CONFLICT => Some("already_exists"),
        reqwest::StatusCode::TOO_MANY_REQUESTS => Some("resource_exhausted"),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR => Some("internal"),
        reqwest::StatusCode::NOT_IMPLEMENTED => Some("unimplemented"),
        reqwest::StatusCode::SERVICE_UNAVAILABLE => Some("unavailable"),
        _ => None,
    }
}
