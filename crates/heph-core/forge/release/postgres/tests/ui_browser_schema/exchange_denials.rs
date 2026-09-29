use super::*;
use crate::exchange::ExchangeContext;

#[allow(clippy::too_many_lines)]
// This phase keeps the original database assertions together as one scenario.
#[allow(clippy::cognitive_complexity)]
pub async fn run(ctx: &ExchangeContext) {
    let worker = ctx.worker.clone();
    let store = &ctx.store;
    let fixture = ctx.fixture;
    let actor = ctx.actor;
    let parent = ctx.parent;
    let installation = ctx.installation;
    let generation = ctx.generation;
    let route = ctx.route.clone();

    // A handoff outside its fixed window is rejected after current authority
    // checks and leaves no child.
    let expired_handoff = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_browser_handoffs
     (id, handoff_digest, request_id, actor_id, parent_session_id,
      installation_id, generation_id, organization_id, route,
      issued_at, expires_at)
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
             statement_timestamp() - interval '61 seconds',
             statement_timestamp() - interval '1 second')",
    )
    .bind(expired_handoff)
    .bind(
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 49))
            .digest()
            .as_bytes()
            .as_slice(),
    )
    .bind(Uuid::new_v4())
    .bind(fixture.actor)
    .bind(fixture.parent_session)
    .bind(fixture.installation)
    .bind(fixture.generation)
    .bind(fixture.organization)
    .bind("schema-ui")
    .execute(&worker)
    .await
    .expect("seed expired handoff");
    let expired = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: RequestId::new(),
            handoff_secret: UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 49)),
            expected_generation_id: generation,
            child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 50)),
        })
        .await;
    assert_eq!(expired, Err(UiBrowserHandoffError::InvalidOrExpired));
    assert_exchange_denial_unchanged(
        &worker,
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 49)),
    )
    .await;

    // Current generation and source publication are rechecked at exchange.
    let stale_secret = UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 51));
    issue_handoff(
        store,
        actor,
        parent,
        installation,
        generation,
        route,
        stale_secret,
    )
    .await;
    let stale_generation = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_installation_generations
     (id, installation_id, generation_no, release_id, ui_key, ui_scope)
     SELECT $1, installation_id, generation_no + 1, release_id, ui_key, ui_scope
     FROM ui_installation_generations WHERE id = $2",
    )
    .bind(stale_generation)
    .bind(fixture.generation)
    .execute(&worker)
    .await
    .expect("seed current generation replacement");
    sqlx::query("UPDATE ui_installations SET current_generation_id = $2 WHERE id = $1")
        .bind(fixture.installation)
        .bind(stale_generation)
        .execute(&worker)
        .await
        .expect("activate current generation replacement");
    let stale = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: RequestId::new(),
            handoff_secret: UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 51)),
            expected_generation_id: generation,
            child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 52)),
        })
        .await;
    assert_eq!(stale, Err(UiBrowserHandoffError::InvalidOrExpired));
    assert_exchange_denial_unchanged(
        &worker,
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 51)),
    )
    .await;

    // Parent revocation/account state is checked before child creation.
    let revoked_secret = UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 53));
    issue_handoff(
        store,
        actor,
        parent,
        UiInstallationId::from_uuid(fixture.other_installation),
        UiInstallationGenerationId::from_uuid(fixture.other_generation),
        UiBrowserRoute::parse("schema-ui-two").expect("second published route base"),
        revoked_secret,
    )
    .await;
    sqlx::query(
        "UPDATE human_browser_sessions
     SET revoked_at = statement_timestamp(), revocation_reason = 'logout'
     WHERE id = $1",
    )
    .bind(fixture.parent_session)
    .execute(&worker)
    .await
    .expect("revoke parent session");
    let revoked = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: RequestId::new(),
            handoff_secret: UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 53)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 54)),
        })
        .await;
    assert_eq!(revoked, Err(UiBrowserHandoffError::InvalidOrExpired));
    assert_exchange_denial_unchanged(
        &worker,
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 53)),
    )
    .await;

    let account_parent = Uuid::new_v4();
    insert_canonical_session(&worker, account_parent, fixture.actor, Uuid::new_v4(), 20).await;
    issue_handoff(
        store,
        actor,
        BrowserSessionId::from_uuid(account_parent),
        UiInstallationId::from_uuid(fixture.other_installation),
        UiInstallationGenerationId::from_uuid(fixture.other_generation),
        UiBrowserRoute::parse("schema-ui-two").expect("second published route base"),
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 57)),
    )
    .await;
    sqlx::query("UPDATE users SET status = 'suspended' WHERE id = $1")
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("suspend account");
    let account_suspended = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: RequestId::new(),
            handoff_secret: UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 57)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 58)),
        })
        .await;
    assert_eq!(
        account_suspended,
        Err(UiBrowserHandoffError::InvalidOrExpired)
    );
    assert_exchange_denial_unchanged(
        &worker,
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 57)),
    )
    .await;
    sqlx::query("UPDATE users SET status = 'active' WHERE id = $1")
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("restore account");

    let source_parent = Uuid::new_v4();
    insert_canonical_session(&worker, source_parent, fixture.actor, Uuid::new_v4(), 20).await;
    issue_handoff(
        store,
        actor,
        BrowserSessionId::from_uuid(source_parent),
        UiInstallationId::from_uuid(fixture.other_installation),
        UiInstallationGenerationId::from_uuid(fixture.other_generation),
        UiBrowserRoute::parse("schema-ui-two").expect("second published route base"),
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 55)),
    )
    .await;
    sqlx::query(
        "UPDATE releases SET state = 'revoked', revoked_at = statement_timestamp()
     WHERE id = (SELECT release_id FROM ui_installation_generations WHERE id = $1)",
    )
    .bind(fixture.other_generation)
    .execute(&worker)
    .await
    .expect("revoke source release");
    let source_revoked = store
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id: RequestId::new(),
            handoff_secret: UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 55)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            child_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 56)),
        })
        .await;
    assert_eq!(source_revoked, Err(UiBrowserHandoffError::InvalidOrExpired));
    assert_exchange_denial_unchanged(
        &worker,
        UiBrowserHandoffSecret::from_bytes(test_secret(fixture.actor, 55)),
    )
    .await;
}
