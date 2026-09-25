use super::{
    ui_installation_scenario_state::{InstallScenarioState, ReleaseClient},
    ui_installation_seed::Fixture,
    ui_installation_transport::{
        HANDOFF_SENTINEL, HANDOFF_SENTINEL_TEXT, InvalidSecretAudit, authorization, opaque,
        request_context,
    },
};
use buffa::Message as _;
use connectrpc::error::ErrorCode;
use rpc_proto::messages::hephaestus::{
    common::v1::RequestContext, release::v1::CreateUiBrowserHandoffRequest,
};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

pub(crate) async fn run(
    pool: &PgPool,
    release: &ReleaseClient,
    fixture: &Fixture,
    app: &hephaestus_app::RunningHephaestus,
    state: &InstallScenarioState,
) {
    let handoff_token = state.handoff_token.clone();
    let installed = &state.installed;
    let secret = HANDOFF_SENTINEL.to_vec();
    let handoffs_before_invalid_secret: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
            .fetch_one(pool)
            .await
            .expect("count handoffs before invalid secret");
    let audit_before_invalid_secret: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_request_audit_events
      WHERE surface = 'handoff_issue'",
    )
    .fetch_one(pool)
    .await
    .expect("count handoff audit rows before invalid secret");
    let invalid_secret_request_id = Uuid::new_v4();
    let invalid_secret = release
        .create_ui_browser_handoff_with_options(
            CreateUiBrowserHandoffRequest {
                context: RequestContext {
                    request_id: opaque(invalid_secret_request_id).into(),
                    ..Default::default()
                }
                .into(),
                installation_id: installed.installation_id.clone(),
                generation_id: installed.generation_id.clone(),
                route: String::from("assistant"),
                handoff_secret: vec![0x5a; 31],
                ..Default::default()
            },
            authorization(&handoff_token),
        )
        .await
        .expect_err("invalid handoff secret must be rejected by production handler");
    assert_eq!(invalid_secret.code, ErrorCode::InvalidArgument);
    let handoffs_after_invalid_secret: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
            .fetch_one(pool)
            .await
            .expect("count handoffs after invalid secret");
    assert_eq!(
        handoffs_after_invalid_secret, handoffs_before_invalid_secret,
        "invalid secret must not create a handoff row"
    );
    let audit_after_invalid_secret: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_request_audit_events
      WHERE surface = 'handoff_issue'",
    )
    .fetch_one(pool)
    .await
    .expect("count handoff audit rows after invalid secret");
    assert_eq!(
        audit_after_invalid_secret,
        audit_before_invalid_secret + 1,
        "invalid secret must create exactly one handoff denial audit"
    );
    let invalid_secret_audit: InvalidSecretAudit = sqlx::query_as(
        "SELECT request_id, decision, outcome, reason_code,
            actor_id, organization_id, installation_id, generation_id
       FROM ui_request_audit_events
      WHERE surface = 'handoff_issue'
      ORDER BY occurred_at DESC, id DESC
      LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("load invalid secret audit row");
    assert_eq!(invalid_secret_audit.1, "denied");
    assert_eq!(invalid_secret_audit.2, "not_attempted");
    assert_eq!(invalid_secret_audit.3, "invalid_input");
    assert_eq!(invalid_secret_audit.4, Some(fixture.user_id));
    assert_eq!(invalid_secret_audit.5, None);
    assert_eq!(invalid_secret_audit.6, None);
    assert_eq!(invalid_secret_audit.7, None);
    assert_ne!(invalid_secret_audit.0, Uuid::nil());
    assert_ne!(
        invalid_secret_audit.0, invalid_secret_request_id,
        "audit correlation must use the server marker, not caller input"
    );

    let handoffs_before_malformed_wire: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
            .fetch_one(pool)
            .await
            .expect("count handoffs before malformed wire request");
    let audit_before_malformed_wire: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_request_audit_events
      WHERE surface = 'handoff_issue'",
    )
    .fetch_one(pool)
    .await
    .expect("count handoff audits before malformed wire request");
    let malformed_wire = reqwest::Client::new()
        .post(format!(
            "http://{}/hephaestus.release.v1.ReleaseService/CreateUiBrowserHandoff",
            app.http_addr()
        ))
        .header("authorization", format!("Bearer {handoff_token}"))
        .header("content-type", "application/proto")
        .body(vec![0xff, 0x00, 0x7f])
        .send()
        .await
        .expect("send malformed handoff wire request");
    let malformed_wire_status = malformed_wire.status();
    let _ = malformed_wire
        .bytes()
        .await
        .expect("read malformed handoff wire response");
    assert!(
        malformed_wire_status.is_client_error(),
        "malformed wire request must retain a client error status"
    );
    let handoffs_after_malformed_wire: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
            .fetch_one(pool)
            .await
            .expect("count handoffs after malformed wire request");
    assert_eq!(
        handoffs_after_malformed_wire, handoffs_before_malformed_wire,
        "malformed wire request must not create a handoff row"
    );
    let audit_after_malformed_wire: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_request_audit_events
      WHERE surface = 'handoff_issue'",
    )
    .fetch_one(pool)
    .await
    .expect("count handoff audits after malformed wire request");
    assert_eq!(
        audit_after_malformed_wire,
        audit_before_malformed_wire + 1,
        "malformed wire request must create exactly one transport denial audit"
    );
    let malformed_wire_audit: (String, String, String, Option<Uuid>) = sqlx::query_as(
        "SELECT decision, outcome, reason_code, actor_id
       FROM ui_request_audit_events
      WHERE surface = 'handoff_issue'
      ORDER BY occurred_at DESC, id DESC
      LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("load malformed wire audit row");
    assert_eq!(malformed_wire_audit.0, "denied");
    assert_eq!(malformed_wire_audit.1, "not_attempted");
    assert_eq!(malformed_wire_audit.2, "invalid_input");
    assert_eq!(malformed_wire_audit.3, Some(fixture.user_id));
    let valid_handoff_request_id = Uuid::new_v4();
    let handoff = release
        .create_ui_browser_handoff_with_options(
            CreateUiBrowserHandoffRequest {
                context: RequestContext {
                    request_id: opaque(valid_handoff_request_id).into(),
                    ..Default::default()
                }
                .into(),
                installation_id: installed.installation_id.clone(),
                generation_id: installed.generation_id.clone(),
                route: String::from("assistant"),
                handoff_secret: secret.clone(),
                ..Default::default()
            },
            authorization(&handoff_token),
        )
        .await
        .expect("authenticated UI handoff")
        .into_owned();
    assert!(handoff.handoff_id.as_option().is_some());
    assert_eq!(handoff.route, "assistant");
    let handoff_id = Uuid::parse_str(&handoff.handoff_id.as_option().expect("handoff ID").value)
        .expect("canonical handoff ID");
    let stored_handoff_request_id: Uuid =
        sqlx::query_scalar("SELECT request_id FROM ui_browser_handoffs WHERE id = $1")
            .bind(handoff_id)
            .fetch_one(pool)
            .await
            .expect("load handoff correlation ID");
    assert_ne!(
        stored_handoff_request_id, valid_handoff_request_id,
        "worker store must receive the server marker, not caller input"
    );
    let handoff_debug = format!("{handoff:?}");
    assert!(!handoff_debug.contains(HANDOFF_SENTINEL_TEXT));
    assert!(!handoff_debug.contains(&fixture.parent_session_id.to_string()));
    let handoff_bytes = handoff.encode_to_vec();
    assert!(
        !handoff_bytes
            .windows(HANDOFF_SENTINEL.len())
            .any(|window| window == HANDOFF_SENTINEL)
    );
    let persisted_handoff_json: String = sqlx::query_scalar(
        "SELECT row_to_json(ui_browser_handoffs)::text
       FROM ui_browser_handoffs
      WHERE id = $1",
    )
    .bind(handoff_id)
    .fetch_one(pool)
    .await
    .expect("load persisted handoff JSON");
    assert!(!persisted_handoff_json.contains(HANDOFF_SENTINEL_TEXT));
    let handoff_events: String = sqlx::query_scalar(
        "SELECT COALESCE(string_agg(row_to_json(application_events)::text, ' '), '')
       FROM application_events
      WHERE request_id = $1",
    )
    .bind(stored_handoff_request_id)
    .fetch_one(pool)
    .await
    .expect("load handoff application event rows");
    assert!(!handoff_events.contains(HANDOFF_SENTINEL_TEXT));

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
            authorization(&handoff_token),
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
            authorization(&handoff_token),
        )
        .await
        .expect_err("handoff idempotency key must be rejected");
    assert_eq!(denied.code, ErrorCode::InvalidArgument);
}
