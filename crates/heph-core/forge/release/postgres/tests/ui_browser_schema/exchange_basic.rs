use super::*;
use crate::exchange::ExchangeContext;

#[allow(clippy::too_many_lines)]
pub async fn run(ctx: &ExchangeContext) {
    let bootstrap = ctx.bootstrap.clone();
    let worker = ctx.worker.clone();
    let store = &ctx.store;
    let fixture = ctx.fixture;
    let actor = ctx.actor;
    let parent = ctx.parent;
    let installation = ctx.installation;
    let generation = ctx.generation;
    let route = ctx.route.clone();

    // A host resolved to another generation cannot exchange the locked handoff.
    let wrong_host_secret = UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 41));
    issue_handoff(
        store,
        actor,
        parent,
        installation,
        generation,
        route.clone(),
        wrong_host_secret,
    )
    .await;
    let wrong_host_request_id = RequestId::new();
    let wrong_host = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: wrong_host_request_id,
            handoff_secret: UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 41)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 42)),
        })
        .await;
    assert_eq!(wrong_host, Err(UiBrowserHandoffError::InvalidOrExpired));
    assert_exchange_denial_unchanged(
        &worker,
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 41)),
    )
    .await;
    assert_audit_row(
        &bootstrap,
        wrong_host_request_id,
        "handoff_exchange",
        "denied",
        "not_attempted",
        "expired",
    )
    .await;
    assert_audit_anonymous(&bootstrap, wrong_host_request_id, "handoff_exchange").await;

    let success_secret = UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 43));
    issue_handoff(
        store,
        actor,
        parent,
        installation,
        generation,
        route.clone(),
        success_secret,
    )
    .await;
    let success_request_id = RequestId::new();
    let success = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: success_request_id,
            handoff_secret: UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 43)),
            expected_generation_id: generation,
            child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 44)),
        })
        .await
        .expect("valid handoff exchanges once");
    let child_row: (
        Vec<u8>,
        time::OffsetDateTime,
        time::OffsetDateTime,
        Uuid,
        Uuid,
    ) = sqlx::query_as(
        "SELECT session_digest, issued_at, expires_at, parent_session_id, generation_id
     FROM ui_browser_sessions WHERE id = $1",
    )
    .bind(success.context.session_id.as_uuid())
    .fetch_one(&worker)
    .await
    .expect("read child safe metadata");
    assert_eq!(
        child_row.0,
        UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 44))
            .digest()
            .as_bytes()
    );
    assert_eq!(child_row.3, fixture.parent_session);
    assert_eq!(child_row.4, fixture.generation);
    assert_eq!(child_row.2 - child_row.1, time::Duration::hours(12));
    assert_audit_row(
        &bootstrap,
        success_request_id,
        "handoff_exchange",
        "allowed",
        "succeeded",
        "none",
    )
    .await;
    assert_audit_context(
        &bootstrap,
        success_request_id,
        "handoff_exchange",
        fixture.actor,
        fixture.organization,
        fixture.installation,
        fixture.generation,
        Some(success.context.session_id.as_uuid()),
    )
    .await;
    let replay_request_id = RequestId::new();
    let replay = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: replay_request_id,
            handoff_secret: UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 43)),
            expected_generation_id: generation,
            child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 45)),
        })
        .await;
    assert_eq!(replay, Err(UiBrowserHandoffError::InvalidOrExpired));
    let replay_children: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_browser_sessions WHERE handoff_id =
     (SELECT id FROM ui_browser_handoffs WHERE handoff_digest = $1)",
    )
    .bind(
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 43))
            .digest()
            .as_bytes()
            .as_slice(),
    )
    .fetch_one(&worker)
    .await
    .expect("count replay children");
    assert_eq!(replay_children, 1);
    assert_audit_row(
        &bootstrap,
        replay_request_id,
        "handoff_exchange",
        "denied",
        "not_attempted",
        "expired",
    )
    .await;
    assert_audit_anonymous(&bootstrap, replay_request_id, "handoff_exchange").await;
}
