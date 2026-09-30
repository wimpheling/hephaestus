use super::*;
use crate::issue::IssueContext;

#[allow(clippy::too_many_lines)]
// This phase keeps the original database assertions together as one scenario.
#[allow(clippy::cognitive_complexity)]
pub async fn run(ctx: &mut IssueContext) {
    let worker = ctx.worker.clone();
    let store = &ctx.store;
    let fixture = ctx.fixture;
    let actor = ctx.actor;
    let parent = ctx.parent;
    let installation = ctx.installation;
    let generation = ctx.generation;
    let route = ctx.route.clone();

    let revoked_parent_id = Uuid::new_v4();
    insert_canonical_session(
        &worker,
        revoked_parent_id,
        fixture.actor,
        Uuid::new_v4(),
        20,
    )
    .await;
    sqlx::query(
        "UPDATE human_browser_sessions
     SET revoked_at = statement_timestamp(), revocation_reason = 'administrative'
     WHERE id = $1",
    )
    .bind(revoked_parent_id)
    .execute(&worker)
    .await
    .expect("revoke parent session");
    let revoked_parent = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: BrowserSessionId::from_uuid(revoked_parent_id),
            installation_id: installation,
            generation_id: generation,
            route: route.clone(),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(revoked_parent, Err(UiBrowserHandoffError::PermissionDenied));

    sqlx::query("UPDATE users SET status = 'suspended' WHERE id = $1")
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("suspend actor account");
    let inactive_actor = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: route.clone(),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(inactive_actor, Err(UiBrowserHandoffError::PermissionDenied));
    sqlx::query("UPDATE users SET status = 'active' WHERE id = $1")
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("restore actor account");

    let future_parent = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO human_browser_sessions
     (id, sid_digest, creation_idempotency_id, creation_request_id,
      identity_binding_digest, user_id, issued_at, expires_at)
     VALUES ($1, $2, $3, $4, $5, $6, statement_timestamp() + interval '1 hour',
             statement_timestamp() + interval '2 hours')",
    )
    .bind(future_parent)
    .bind(digest(224))
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest(225))
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("seed future-issued parent");
    let future_issued = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: BrowserSessionId::from_uuid(future_parent),
            installation_id: installation,
            generation_id: generation,
            route: route.clone(),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(future_issued, Err(UiBrowserHandoffError::PermissionDenied));

    let actor_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: UserId::from_uuid(fixture.outsider),
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: UiBrowserRoute::parse("schema-ui").expect("published route base"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(actor_denied, Err(UiBrowserHandoffError::PermissionDenied));

    sqlx::query("UPDATE ui_installations SET lifecycle = 'disabled' WHERE id = $1")
        .bind(fixture.installation)
        .execute(&worker)
        .await
        .expect("disable installation for denial case");
    let disabled_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: UiBrowserRoute::parse("schema-ui").expect("published route base"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(
        disabled_denied,
        Err(UiBrowserHandoffError::PermissionDenied)
    );
    sqlx::query("UPDATE ui_installations SET lifecycle = 'enabled' WHERE id = $1")
        .bind(fixture.installation)
        .execute(&worker)
        .await
        .expect("restore installation for generation case");

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
    .expect("seed newer generation");
    sqlx::query("UPDATE ui_installations SET current_generation_id = $2 WHERE id = $1")
        .bind(fixture.installation)
        .bind(stale_generation)
        .execute(&worker)
        .await
        .expect("activate newer generation");
    let stale_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: generation,
            route: UiBrowserRoute::parse("schema-ui").expect("published route base"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(stale_denied, Err(UiBrowserHandoffError::PermissionDenied));

    let expiring_parent = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO human_browser_sessions
     (id, sid_digest, creation_idempotency_id, creation_request_id,
      identity_binding_digest, user_id, issued_at, expires_at)
     VALUES ($1, $2, $3, $4, $5, $6, statement_timestamp(),
             statement_timestamp() + interval '1 second')",
    )
    .bind(expiring_parent)
    .bind(digest(220))
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest(221))
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("seed short-lived parent");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let expired_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: BrowserSessionId::from_uuid(expiring_parent),
            installation_id: installation,
            generation_id: UiInstallationGenerationId::from_uuid(stale_generation),
            route: UiBrowserRoute::parse("schema-ui").expect("published route base"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(expired_denied, Err(UiBrowserHandoffError::PermissionDenied));

    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("revoke actor target authority");
    let target_revoked = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: UiInstallationId::from_uuid(fixture.global_installation),
            generation_id: UiInstallationGenerationId::from_uuid(fixture.global_generation),
            route: UiBrowserRoute::parse("schema-global").expect("global route"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(target_revoked, Err(UiBrowserHandoffError::PermissionDenied));
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
     VALUES ($1, $2, 'owner') ON CONFLICT DO NOTHING",
    )
    .bind(fixture.organization)
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("restore actor target authority");
    ctx.stale_generation = stale_generation;
}
