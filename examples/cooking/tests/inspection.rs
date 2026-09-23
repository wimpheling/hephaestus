//! Authenticated HTTP inspection of the real mailbox cooking run.

use identity_domain::BrowserSessionSid;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

// Keep the owner and outsider identities explicit at this boundary so each
// negative probe uses a real, independently seeded browser session.
#[allow(clippy::too_many_arguments)]
pub async fn inspect(
    pool: &PgPool,
    running: &hephaestus_app::RunningHephaestus,
    run_id: Uuid,
    event_id: Uuid,
    approve_result: bool,
    owner_browser_session: BrowserSessionSid,
    outsider: Uuid,
    outsider_browser_session: BrowserSessionSid,
) -> Value {
    inspect_with_https_uses(
        pool,
        running,
        run_id,
        event_id,
        approve_result,
        4,
        owner_browser_session,
        outsider,
        outsider_browser_session,
    )
    .await
}

/// Inspects a run whose controlled fault path may repeat one broker call.
/// The expected count remains explicit so the happy path keeps its exact
/// four-use assertion while retries prove their additional physical use.
// Explicit owner/outsider sessions keep the authorization probes independent.
#[allow(clippy::too_many_arguments)]
pub async fn inspect_with_https_uses(
    pool: &PgPool,
    running: &hephaestus_app::RunningHephaestus,
    run_id: Uuid,
    event_id: Uuid,
    approve_result: bool,
    expected_https_uses: usize,
    owner_browser_session: BrowserSessionSid,
    outsider: Uuid,
    outsider_browser_session: BrowserSessionSid,
) -> Value {
    let owner: Uuid = sqlx::query_scalar(
        "SELECT user_id FROM external_identities WHERE issuer = $1 AND subject = 'golden-subject'",
    )
    .bind(super::golden_issuer())
    .fetch_one(pool)
    .await
    .expect("golden inspection owner");
    let client = reqwest::Client::new();
    let mut provenance = Value::Null;
    let mut result_run = Value::Null;
    inspect_source(
        pool,
        running,
        &client,
        owner,
        run_id,
        event_id,
        owner_browser_session,
    )
    .await;
    for method in ["GetRun", "GetRunProvenance"] {
        let audience = format!("/hephaestus.run.v1.RunService/{method}");
        let url = format!("http://{}{audience}", running.http_addr());
        let request = json!({"runId":{"value":run_id.to_string()}});
        let response = client
            .post(&url)
            .bearer_auth(assertion(owner, &audience, owner_browser_session))
            .json(&request)
            .send()
            .await
            .expect("owner inspection RPC");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let body: Value = response.json().await.expect("typed JSON inspection");
        if method == "GetRun" {
            assert_eq!(body["run"]["id"]["value"], run_id.to_string());
            assert!(
                !body["run"]["result"]["commit"]
                    .as_str()
                    .unwrap_or_default()
                    .is_empty()
            );
            assert_eq!(body["run"]["gitRef"], "refs/heads/main");
            result_run = body["run"].clone();
        } else {
            provenance = body;
        }
        let outsider = client
            .post(&url)
            .bearer_auth(assertion(outsider, &audience, outsider_browser_session))
            .json(&request)
            .send()
            .await
            .expect("outsider inspection RPC");
        assert_eq!(outsider.status(), reqwest::StatusCode::NOT_FOUND);
        let wrong_audience = client
            .post(&url)
            .bearer_auth(assertion(owner, "/wrong-audience", owner_browser_session))
            .json(&request)
            .send()
            .await
            .expect("wrong-audience inspection RPC");
        assert_eq!(wrong_audience.status(), reqwest::StatusCode::UNAUTHORIZED);
    }
    assert!(
        !provenance["authorizationSnapshotId"]["value"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );
    inspect_https(pool, run_id, &provenance, expected_https_uses).await;
    assert!(
        !provenance
            .to_string()
            .contains(super::BROKERED_E2E_SENTINEL)
    );
    super::cooking::assert_no_credentials(&provenance.to_string());
    if approve_result {
        approve(
            pool,
            running,
            &client,
            owner,
            owner_browser_session,
            &result_run,
        )
        .await;
    }
    result_run
}

async fn inspect_https(
    pool: &PgPool,
    run_id: Uuid,
    provenance: &Value,
    expected_https_uses: usize,
) {
    let uses = provenance["httpsUses"]
        .as_array()
        .expect("visible HTTPS evidence");
    assert_eq!(uses.len(), expected_https_uses);
    let mut rules = std::collections::BTreeSet::new();
    for usage in uses {
        for field in [
            "leaseId",
            "bindingId",
            "secretVersionId",
            "ruleId",
            "requestId",
        ] {
            assert!(
                Uuid::parse_str(usage[field]["value"].as_str().expect("exact opaque ID")).is_ok()
            );
        }
        rules.insert(usage["ruleId"]["value"].as_str().expect("rule ID"));
        let expected: (Uuid, Uuid, Uuid) = sqlx::query_as(
            "SELECT lease_id,binding_id,secret_version_id FROM brokered_secret_lease_snapshots WHERE run_id=$1 AND rule_id=$2",
        ).bind(run_id).bind(Uuid::parse_str(usage["ruleId"]["value"].as_str().expect("rule")).expect("rule UUID"))
            .fetch_one(pool).await.expect("exact immutable lease evidence");
        assert_eq!(usage["leaseId"]["value"], expected.0.to_string());
        assert_eq!(usage["bindingId"]["value"], expected.1.to_string());
        assert_eq!(usage["secretVersionId"]["value"], expected.2.to_string());
    }
    let expected_rules = if expected_https_uses == 2 {
        std::collections::BTreeSet::from(["00000000-0000-0000-0000-000000000004"])
    } else {
        std::collections::BTreeSet::from([
            "00000000-0000-0000-0000-000000000003",
            "00000000-0000-0000-0000-000000000004",
        ])
    };
    assert_eq!(rules, expected_rules);
}

async fn inspect_source(
    pool: &PgPool,
    running: &hephaestus_app::RunningHephaestus,
    client: &reqwest::Client,
    owner: Uuid,
    run_id: Uuid,
    event_id: Uuid,
    owner_browser_session: BrowserSessionSid,
) {
    let source: (Uuid, Uuid, Uuid, Uuid) = sqlx::query_as(
        "SELECT run.instance_id,delivery.event_id,revision.gateway_id,lease.id
         FROM runs run JOIN mailbox_delivery_attempts delivery ON delivery.run_id=run.id
         JOIN gateway_mailbox_publications publication ON publication.event_id=delivery.event_id
         JOIN gateway_revisions revision ON revision.id=publication.gateway_revision_id
         JOIN agent_instance_volume_leases lease ON lease.run_id=run.id WHERE run.id=$1",
    )
    .bind(run_id)
    .fetch_one(pool)
    .await
    .expect("exact ingress and state chain");
    assert_eq!(
        source.1, event_id,
        "inspection must resolve the requested event"
    );
    let gateway = get_json(
        client,
        running,
        owner,
        "/hephaestus.gateway.v1.GatewayService/GetGateway",
        json!({"gatewayId":{"value":source.2.to_string()}}),
        owner_browser_session,
    )
    .await;
    assert!(
        gateway["revisions"]
            .as_array()
            .expect("gateway revisions")
            .iter()
            .any(
                |revision| revision["routes"].as_array().is_some_and(|routes| routes
                    .iter()
                    .any(|route| route["path"] == "/cooking/telegram"
                        || route["path"] == "/gateway/cooking/telegram"))
            )
    );
    let publications = get_json(
        client,
        running,
        owner,
        "/hephaestus.gateway.v1.GatewayService/ListMailboxPublications",
        json!({"gatewayId":{"value":source.2.to_string()}}),
        owner_browser_session,
    )
    .await;
    let publication = publications
        .get("publications")
        .and_then(Value::as_array)
        .and_then(|publications| {
            publications
                .iter()
                .find(|publication| publication["runId"]["value"] == run_id.to_string())
        });
    let Some(publication) = publication else {
        emit_publication_diagnostic(pool, &publications, source.2, event_id, run_id).await;
        panic!("accepted invocation to run");
    };
    assert_eq!(publication["eventId"]["value"], source.1.to_string());
    assert_eq!(publication["runOutcome"], "succeeded");
    let instance = get_json(
        client,
        running,
        owner,
        "/hephaestus.instance.v1.AgentInstanceService/GetInstance",
        json!({"instanceId":{"value":source.0.to_string()}}),
        owner_browser_session,
    )
    .await;
    let delivery = instance["instance"]["mailboxDeliveries"]
        .as_array()
        .expect("mailbox deliveries")
        .iter()
        .find(|delivery| delivery["eventId"]["value"] == source.1.to_string())
        .expect("event state lease");
    assert_eq!(delivery["leaseId"]["value"], source.3.to_string());
    assert!(
        !delivery["stateVolumeId"]["value"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );
}

async fn emit_publication_diagnostic(
    pool: &PgPool,
    response: &Value,
    gateway_id: Uuid,
    event_id: Uuid,
    run_id: Uuid,
) {
    let (api_publications, page_metadata) = api_publication_diagnostic(response);
    let sql_summary = sql_publication_diagnostic(pool, gateway_id, event_id).await;
    eprintln!(
        "cooking publication inspection diagnostic: {}",
        json!({
            "requestedGatewayId": gateway_id,
            "requestedEventId": event_id,
            "requestedRunId": run_id,
            "api": {
                "publicationCount": api_publications.len(),
                "publications": api_publications,
                "page": page_metadata,
            },
            "sql": sql_summary,
        })
    );
}

fn api_publication_diagnostic(response: &Value) -> (Vec<Value>, Value) {
    let api_publications = response
        .get("publications")
        .and_then(Value::as_array)
        .map(|publications| {
            publications
                .iter()
                .map(|publication| {
                    json!({
                        "id": opaque_id(publication, "id"),
                        "eventId": opaque_id(publication, "eventId"),
                        "deliveryAttemptId": opaque_id(publication, "deliveryAttemptId"),
                        "runId": opaque_id(publication, "runId"),
                        "runState": string_field(publication, "runState"),
                        "runOutcome": string_field(publication, "runOutcome"),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let page = response.get("page");
    let has_next_page = page
        .and_then(|page| string_field(page, "nextPageToken"))
        .is_some_and(|token| !token.is_empty());
    let page_metadata = json!({
        "stableOrder": page.and_then(|page| string_field(page, "stableOrder")),
        "hasNextPage": has_next_page,
    });
    (api_publications, page_metadata)
}

type PublicationDiagnosticRow = (
    Uuid,
    Option<Uuid>,
    String,
    OffsetDateTime,
    Option<Uuid>,
    Option<i32>,
    Option<Uuid>,
    Option<String>,
    Option<OffsetDateTime>,
    Option<OffsetDateTime>,
    Option<String>,
    Option<String>,
    Option<OffsetDateTime>,
    Option<OffsetDateTime>,
);

async fn sql_publication_diagnostic(pool: &PgPool, gateway_id: Uuid, event_id: Uuid) -> Value {
    let sql_rows = sqlx::query_as::<_, PublicationDiagnosticRow>(
        "SELECT publication.id, publication.event_id, publication.outcome,
                publication.accepted_at, attempt.id, attempt.attempt_number,
                attempt.run_id, attempt.state, attempt.created_at,
                attempt.completed_at, run.state, run.outcome, run.created_at,
                run.updated_at
           FROM gateway_mailbox_publications publication
           JOIN gateway_invocations invocation
             ON invocation.id = publication.invocation_id
           LEFT JOIN mailbox_delivery_attempts attempt
             ON attempt.event_id = publication.event_id
           LEFT JOIN runs run ON run.id = attempt.run_id
          WHERE invocation.gateway_id = $1 AND publication.event_id = $2
          ORDER BY publication.id DESC, attempt.attempt_number DESC, attempt.id DESC",
    )
    .bind(gateway_id)
    .bind(event_id)
    .fetch_all(pool)
    .await;
    sql_rows.map_or_else(
        |_| json!({"query": "failed"}),
        |rows| {
            let publication_row_count = rows
                .iter()
                .map(|row| row.0)
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            let attempt_row_count = rows.iter().filter(|row| row.4.is_some()).count();
            json!({
                "publicationRowCount": publication_row_count,
                "attemptRowCount": attempt_row_count,
                "rows": rows.into_iter().map(diagnostic_sql_row).collect::<Vec<_>>(),
            })
        },
    )
}

fn diagnostic_sql_row(
    (
        publication_id,
        publication_event_id,
        publication_outcome,
        publication_accepted_at,
        attempt_id,
        attempt_number,
        attempt_run_id,
        attempt_state,
        attempt_created_at,
        attempt_completed_at,
        run_state,
        run_outcome,
        run_created_at,
        run_updated_at,
    ): PublicationDiagnosticRow,
) -> Value {
    json!({
        "publicationId": publication_id,
        "publicationEventId": publication_event_id,
        "publicationOutcome": publication_outcome,
        "publicationAcceptedAt": diagnostic_timestamp(Some(publication_accepted_at)),
        "attemptId": attempt_id,
        "attemptNumber": attempt_number,
        "attemptRunId": attempt_run_id,
        "attemptState": attempt_state,
        "attemptCreatedAt": diagnostic_timestamp(attempt_created_at),
        "attemptCompletedAt": diagnostic_timestamp(attempt_completed_at),
        "runState": run_state,
        "runOutcome": run_outcome,
        "runCreatedAt": diagnostic_timestamp(run_created_at),
        "runUpdatedAt": diagnostic_timestamp(run_updated_at),
    })
}

fn opaque_id(value: &Value, field: &str) -> Value {
    value
        .get(field)
        .and_then(|field| field.get("value"))
        .and_then(Value::as_str)
        .map_or(Value::Null, |value| Value::String(value.to_owned()))
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn diagnostic_timestamp(value: Option<OffsetDateTime>) -> Value {
    value.map_or(Value::Null, |value| {
        json!({
            "unixSeconds": value.unix_timestamp(),
            "nanoseconds": value.nanosecond(),
        })
    })
}

async fn get_json(
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

fn rpc_failure_marker(audience: &str, status: reqwest::StatusCode, body: &[u8]) -> String {
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

fn valid_connect_code(value: &str) -> Option<&'static str> {
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

const fn status_connect_code(status: reqwest::StatusCode) -> Option<&'static str> {
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

fn assertion(user: Uuid, audience: &str, owner_browser_session: BrowserSessionSid) -> String {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let signing_key = hephaestus_app::rpc::mediator_signing_key(
        b"golden-internal-command-token-with-sufficient-entropy",
    );
    encode(
        &Header::new(Algorithm::HS256),
        &json!({
            "iss":"hephaestus-web-mediator", "sub":user.to_string(),
            "aud":audience, "jti":Uuid::new_v4().to_string(),
            "iat":now, "nbf":now, "exp":now+30,
            "sid":owner_browser_session.to_protocol_string(),
        }),
        &EncodingKey::from_secret(&signing_key),
    )
    .expect("sign exact inspection assertion")
}

async fn approve(
    pool: &PgPool,
    running: &hephaestus_app::RunningHephaestus,
    client: &reqwest::Client,
    owner: Uuid,
    owner_browser_session: BrowserSessionSid,
    run: &Value,
) {
    let audience = "/hephaestus.run.v1.RunService/RequestControl";
    let proposal = run["result"]["proposal"]["id"]["value"]
        .as_str()
        .expect("proposal ID");
    let response = client.post(format!("http://{}{audience}", running.http_addr()))
        .bearer_auth(assertion(owner, audience, owner_browser_session))
        .json(&json!({
            "context":{"requestId":{"value":Uuid::new_v4().to_string()}, "idempotencyKey":format!("cooking-approve-{proposal}")},
            "kind":"RUN_CONTROL_KIND_APPROVE_RESULT", "repositoryId":run["repositoryId"],
            "target":{"proposalId":{"value":proposal}}, "reason":"Publish the verified cooking fixture result",
        })).send().await.expect("authorized cooking approval");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let receipt: Value = response.json().await.expect("approval receipt");
    let control = Uuid::parse_str(
        receipt["controlRequestId"]["value"]
            .as_str()
            .expect("control ID"),
    )
    .expect("control UUID");
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let state: String =
                sqlx::query_scalar("SELECT state FROM control_requests WHERE id=$1")
                    .bind(control)
                    .fetch_one(pool)
                    .await
                    .expect("approval disposition");
            assert_ne!(state, "failed", "controlled publication failed");
            if state == "completed" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("controlled publication completes");
}

/// Requests approval for a separately inspected proposal. The caller uses
/// this for a stale competing proposal after canonical Git has advanced.
pub async fn approve_for_test(
    pool: &PgPool,
    running: &hephaestus_app::RunningHephaestus,
    run: &Value,
    owner_browser_session: BrowserSessionSid,
) {
    let owner: Uuid = sqlx::query_scalar(
        "SELECT user_id FROM external_identities WHERE issuer = $1 AND subject = 'golden-subject'",
    )
    .bind(super::golden_issuer())
    .fetch_one(pool)
    .await
    .expect("golden approval owner");
    approve(
        pool,
        running,
        &reqwest::Client::new(),
        owner,
        owner_browser_session,
        run,
    )
    .await;
}

#[cfg(test)]
mod tests {
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
}
