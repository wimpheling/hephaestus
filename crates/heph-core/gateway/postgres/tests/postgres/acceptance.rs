use super::support::{
    request, revoke_grant, seed_dispatch_target, seed_fixture, set_instance_state,
};
/// Acceptance and lifecycle scenario.
use gateway_postgres::{GatewayMailboxPublicationResult, PostgresGatewayMailboxPublisher};
use mailbox_dispatch::{MAILBOX_WAKE_SUBJECT, MailboxDispatchStore};
use mailbox_postgres::PostgresMailboxRepository;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::env;

#[tokio::test(flavor = "multi_thread")]
#[serial]
// Keep acceptance, lifecycle denial, and resumed dispatch in one fixture so
// the test proves that the same accepted work survives the closed gate.
#[allow(clippy::too_many_lines)]
async fn gateway_accepts_during_updates_but_denies_disabled_and_recovery_instances() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .expect("migrate");
    let fixture = seed_fixture(&pool).await;
    seed_dispatch_target(&pool, &fixture).await;
    let publisher = PostgresGatewayMailboxPublisher::new(pool.clone());
    let mut deferred_event = None;
    for state in ["update_draining", "updating"] {
        set_instance_state(&pool, fixture.mailbox, state).await;
        let outcome = publisher
            .publish(request(&fixture, "deliver", state))
            .await
            .expect("accept during update");
        let GatewayMailboxPublicationResult::Accepted { event_id } = outcome else {
            panic!("expected durable acceptance during {state}, got {outcome:?}");
        };
        assert_eq!(
            publisher
                .publish(request(&fixture, "deliver", state))
                .await
                .expect("deduplicate during update"),
            GatewayMailboxPublicationResult::Duplicate { event_id }
        );
        let command: serde_json::Value = sqlx::query_scalar(
            "SELECT payload FROM outbox WHERE id = $1 AND subject = 'heph.mailbox.v1.wake'",
        )
        .bind(event_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("committed wake");
        let command = serde_json::from_value(command).expect("wake command");
        let store = PostgresMailboxRepository::new(pool.clone());
        store
            .apply_command(MAILBOX_WAKE_SUBJECT, &command)
            .await
            .expect("wake accepted work");
        assert!(
            store
                .claim_dispatch(&command)
                .await
                .expect("closed gate defers dispatch")
                .is_none()
        );
        deferred_event = Some(event_id);
        let attempts: i32 = sqlx::query_scalar(
            "SELECT logical_attempt_count FROM mailbox_deliveries WHERE event_id = $1",
        )
        .bind(event_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("deferred delivery");
        assert_eq!(attempts, 0);
    }
    for state in [
        "disabled",
        "paused_unknown_state",
        "paused_activation_recovery",
        "recovering",
        "removed",
    ] {
        set_instance_state(&pool, fixture.mailbox, state).await;
        assert_eq!(
            publisher
                .publish(request(&fixture, "deliver", state))
                .await
                .expect("redacted lifecycle denial"),
            GatewayMailboxPublicationResult::Denied
        );
    }
    set_instance_state(&pool, fixture.mailbox, "updating").await;
    revoke_grant(&pool, fixture.grant, fixture.owner).await;
    assert_eq!(
        publisher
            .publish(request(&fixture, "deliver", "revoked-during-update"))
            .await
            .expect("live grant check during update"),
        GatewayMailboxPublicationResult::Denied
    );
    let events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1")
            .bind(fixture.mailbox)
            .fetch_one(&pool)
            .await
            .expect("only update requests accepted");
    assert_eq!(events, 2);
    sqlx::query("UPDATE agent_instances SET state = 'active', run_gate_open = true WHERE id = (SELECT instance_id FROM mailboxes WHERE id = $1)")
        .bind(fixture.mailbox).execute(&pool).await.expect("reopen gate");
    let event_id = deferred_event.expect("accepted deferred event");
    let command: serde_json::Value = sqlx::query_scalar("SELECT payload FROM outbox WHERE subject = 'heph.mailbox.v1.dispatch' AND payload->>'mailbox_event_id' = $1")
        .bind(event_id.as_uuid().to_string()).fetch_one(&pool).await.expect("committed dispatch");
    let command = serde_json::from_value(command).expect("dispatch command");
    assert!(
        PostgresMailboxRepository::new(pool)
            .claim_dispatch(&command)
            .await
            .expect("dispatch accepted work after reopening")
            .is_some()
    );
}
