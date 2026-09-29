use super::*;
use crate::issue::IssueContext;

#[allow(clippy::too_many_lines)]
// This phase keeps the original database assertions together as one scenario.
#[allow(clippy::cognitive_complexity)]
pub async fn run(ctx: &mut IssueContext) {
    let bootstrap = ctx.bootstrap.clone();
    let worker = ctx.worker.clone();
    let store = &ctx.store;
    let fixture = ctx.fixture;
    let actor = ctx.actor;
    let parent = ctx.parent;
    let installation = ctx.installation;
    let generation = ctx.generation;
    let route = ctx.route.clone();
    let request_id = RequestId::new();
    let secret = UiBrowserHandoffSecret::random();
    let expected_digest = secret.digest().as_bytes();

    let created = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id,
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: route.clone(),
            secret,
        })
        .await
        .expect("active actor may issue for current generation");
    assert_eq!(created.organization_id.as_uuid(), fixture.organization);
    assert_eq!(created.route, route);
    assert_audit_row(
        &bootstrap,
        request_id,
        "handoff_issue",
        "allowed",
        "succeeded",
        "none",
    )
    .await;
    assert_audit_row(
        &bootstrap,
        request_id,
        "embed",
        "allowed",
        "succeeded",
        "none",
    )
    .await;
    assert_audit_context(
        &bootstrap,
        request_id,
        "handoff_issue",
        fixture.actor,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        None,
    )
    .await;
    assert_audit_context(
        &bootstrap,
        request_id,
        "embed",
        fixture.actor,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        None,
    )
    .await;

    let full_page_request_id = RequestId::new();
    store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: full_page_request_id,
            actor_id: actor,
            parent_session_id: parent,
            installation_id: UiInstallationId::from_uuid(fixture.other_installation),
            generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            route: UiBrowserRoute::parse("schema-ui-two").expect("full-page route"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await
        .expect("full-page UI may issue a handoff");
    assert_audit_row(
        &bootstrap,
        full_page_request_id,
        "handoff_issue",
        "allowed",
        "succeeded",
        "none",
    )
    .await;
    let full_page_embeds: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_request_audit_events
     WHERE request_id = $1 AND surface = 'embed'",
    )
    .bind(full_page_request_id.as_uuid())
    .fetch_one(&bootstrap)
    .await
    .expect("count full-page embed audit rows");
    assert_eq!(
        full_page_embeds, 0,
        "full-page UI must not emit embed audit"
    );

    let handoff_count_before_audit_failure: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
            .fetch_one(&bootstrap)
            .await
            .expect("count handoffs before audit failure");
    sqlx::query("REVOKE INSERT ON public.ui_request_audit_events FROM hephaestus_worker")
        .execute(&bootstrap)
        .await
        .expect("revoke audit insert for atomicity test");
    let audit_failure_request_id = RequestId::new();
    let audit_failure = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: audit_failure_request_id,
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: route.clone(),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(audit_failure, Err(UiBrowserHandoffError::Unavailable));
    sqlx::query("GRANT INSERT ON public.ui_request_audit_events TO hephaestus_worker")
        .execute(&bootstrap)
        .await
        .expect("restore audit insert after atomicity test");
    let handoff_count_after_audit_failure: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
            .fetch_one(&bootstrap)
            .await
            .expect("count handoffs after audit failure");
    assert_eq!(
        handoff_count_after_audit_failure, handoff_count_before_audit_failure,
        "audit append failure rolled back the handoff mutation"
    );
    let audit_failure_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_request_audit_events WHERE request_id = $1")
            .bind(audit_failure_request_id.as_uuid())
            .fetch_one(&bootstrap)
            .await
            .expect("count audit failure rows");
    assert_eq!(audit_failure_rows, 0);

    let stored: (
        Vec<u8>,
        Uuid,
        Uuid,
        Uuid,
        Uuid,
        String,
        time::OffsetDateTime,
        time::OffsetDateTime,
    ) = sqlx::query_as(
        "SELECT handoff_digest, request_id, actor_id, parent_session_id,
                organization_id, route, issued_at, expires_at
         FROM ui_browser_handoffs WHERE id = $1",
    )
    .bind(created.handoff_id.as_uuid())
    .fetch_one(&worker)
    .await
    .expect("read issued handoff metadata");
    assert_eq!(
        stored.0, expected_digest,
        "stored digest matches the secret"
    );
    assert_eq!(stored.1, request_id.as_uuid());
    assert_eq!(stored.2, fixture.actor);
    assert_eq!(stored.3, fixture.parent_session);
    assert_eq!(stored.4, fixture.organization);
    assert_eq!(stored.5, "schema-ui");
    assert_eq!(stored.7 - stored.6, time::Duration::seconds(60));

    for (installation_id, generation_id, route_text) in [
        (
            fixture.global_installation,
            fixture.global_generation,
            "schema-global",
        ),
        (
            fixture.repository_installation,
            fixture.repository_generation,
            "schema-repository",
        ),
    ] {
        let scope_secret = UiBrowserHandoffSecret::random();
        let scope_digest = scope_secret.digest().as_bytes();
        let scope_created = store
            .create_ui_browser_handoff(CreateUiBrowserHandoff {
                request_id: RequestId::new(),
                actor_id: actor,
                parent_session_id: parent,
                installation_id: UiInstallationId::from_uuid(installation_id),
                generation_id: UiInstallationGenerationId::from_uuid(generation_id),
                route: UiBrowserRoute::parse(route_text).expect("scope route"),
                secret: scope_secret,
            })
            .await
            .expect("active actor may issue for every owner scope");
        assert_eq!(
            scope_created.organization_id.as_uuid(),
            fixture.organization
        );
        let stored_scope_digest: Vec<u8> =
            sqlx::query_scalar("SELECT handoff_digest FROM ui_browser_handoffs WHERE id = $1")
                .bind(scope_created.handoff_id.as_uuid())
                .fetch_one(&worker)
                .await
                .expect("read scope handoff digest");
        assert_eq!(stored_scope_digest, scope_digest);
    }

    let baseline: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
        .fetch_one(&worker)
        .await
        .expect("count issued handoffs");
    ctx.baseline = baseline;
}
