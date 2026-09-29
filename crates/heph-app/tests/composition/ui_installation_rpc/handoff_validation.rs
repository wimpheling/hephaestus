use super::{
    ui_installation_scenario_state::ReleaseClient,
    ui_installation_transport::{
        HANDOFF_SENTINEL, HANDOFF_SENTINEL_TEXT, authorization, request_context,
    },
};
use connectrpc::error::ErrorCode;
use rpc_proto::messages::hephaestus::release::v1::{
    CreateUiBrowserHandoffRequest, InstallUiResponse,
};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

pub(super) async fn assert_invalid_route(
    pool: &PgPool,
    release: &ReleaseClient,
    installed: &InstallUiResponse,
    handoff_token: &str,
) {
    let invalid_route_context = request_context("invalid-route-secret");
    let invalid_route_request_id = invalid_route_context
        .request_id
        .as_option()
        .expect("invalid-route request ID")
        .value
        .parse::<Uuid>()
        .expect("invalid-route request UUID");
    let invalid_route_started_at: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(pool)
        .await
        .expect("capture invalid-route audit timestamp");
    let handoff_audits_before_invalid_route: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_request_audit_events
      WHERE surface = 'handoff_issue'",
    )
    .fetch_one(pool)
    .await
    .expect("count handoff audits before invalid route");
    let invalid_route = release
        .create_ui_browser_handoff_with_options(
            CreateUiBrowserHandoffRequest {
                context: invalid_route_context.into(),
                installation_id: installed.installation_id.clone(),
                generation_id: installed.generation_id.clone(),
                route: String::from("invalid route"),
                handoff_secret: HANDOFF_SENTINEL.to_vec(),
                ..Default::default()
            },
            authorization(handoff_token),
        )
        .await
        .expect_err("invalid route must reject a parsed handoff secret");
    assert_eq!(invalid_route.code, ErrorCode::InvalidArgument);
    assert!(!invalid_route.to_string().contains(HANDOFF_SENTINEL_TEXT));
    assert!(!format!("{invalid_route:?}").contains(HANDOFF_SENTINEL_TEXT));
    let handoff_audits_after_invalid_route: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_request_audit_events
      WHERE surface = 'handoff_issue'",
    )
    .fetch_one(pool)
    .await
    .expect("count handoff audits after invalid route");
    assert_eq!(
        handoff_audits_after_invalid_route,
        handoff_audits_before_invalid_route + 1,
        "invalid route must append exactly one denial audit"
    );
    let (invalid_route_audit_request_id, invalid_route_audit_reason): (Uuid, String) =
        sqlx::query_as(
            "SELECT request_id, reason_code
           FROM ui_request_audit_events
          WHERE surface = 'handoff_issue'
            AND occurred_at >= $1
          ORDER BY occurred_at ASC, id ASC
          LIMIT 1",
        )
        .bind(invalid_route_started_at)
        .fetch_one(pool)
        .await
        .expect("load invalid-route handoff audit provenance");
    assert_eq!(invalid_route_audit_reason, "invalid_input");
    assert_ne!(
        invalid_route_audit_request_id, invalid_route_request_id,
        "handoff middleware must use its generated audit marker"
    );
    let caller_audit_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_request_audit_events
      WHERE surface = 'handoff_issue' AND request_id = $1",
    )
    .bind(invalid_route_request_id)
    .fetch_one(pool)
    .await
    .expect("query caller request audit provenance");
    assert_eq!(caller_audit_rows, 0);
    let invalid_route_audit_json: String = sqlx::query_scalar(
        "SELECT row_to_json(ui_request_audit_events)::text
       FROM ui_request_audit_events
      WHERE surface = 'handoff_issue' AND request_id = $1
        AND reason_code = 'invalid_input'
        AND occurred_at >= $2",
    )
    .bind(invalid_route_audit_request_id)
    .bind(invalid_route_started_at)
    .fetch_one(pool)
    .await
    .expect("load invalid-route handoff audit JSON");
    assert!(!invalid_route_audit_json.contains(HANDOFF_SENTINEL_TEXT));
}

pub(super) async fn assert_empty_idempotency(
    release: &ReleaseClient,
    installed: &InstallUiResponse,
    handoff_token: &str,
) {
    let secret = HANDOFF_SENTINEL.to_vec();
    let denied = release
        .create_ui_browser_handoff_with_options(
            CreateUiBrowserHandoffRequest {
                context: request_context("must-be-empty").into(),
                installation_id: installed.installation_id.clone(),
                generation_id: installed.generation_id.clone(),
                route: String::from("assistant"),
                handoff_secret: secret,
                ..Default::default()
            },
            authorization(handoff_token),
        )
        .await
        .expect_err("handoff idempotency key must be rejected");
    assert_eq!(denied.code, ErrorCode::InvalidArgument);
}
