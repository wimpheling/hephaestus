/// configuration scenario.
use super::support::seed_fixture;
use authz_postgres::PostgresMelangeAuthorizer;
use gateway_postgres::{ConfigureGatewayRequest, GatewayConfigureError, PostgresGatewayManagement};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::{collections::BTreeMap, env, sync::Arc};
use uuid::Uuid;

#[tokio::test]
#[serial]
async fn gateway_configuration_is_immutable_authorized_replayable_and_fail_closed() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&database_url)
        .await
        .expect("connect configuration PostgreSQL");
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .expect("apply configuration migrations");
    let fixture = seed_fixture(&pool).await;
    let owner = AuthenticatedIdentity::new(
        UserId::from_uuid(fixture.owner),
        "test",
        "gateway-configure-owner",
        serde_json::json!({}),
        RequestId::new(),
    );
    let management =
        PostgresGatewayManagement::new(pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let first = management
        .configure(
            &owner,
            ConfigureGatewayRequest {
                gateway_id: fixture.gateway,
                expected_revision_id: fixture.revision,
                parameters: BTreeMap::new(),
                secret_selections: Vec::new(),
            },
        )
        .await
        .expect("owner configures published gateway");
    assert_ne!(first.revision_id, fixture.revision);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM gateway_mailbox_bindings WHERE gateway_revision_id = $1",
        )
        .bind(first.revision_id)
        .fetch_one(&pool)
        .await
        .expect("count new revision bindings"),
        0,
        "configuration must not copy mailbox grants"
    );
    let second = management
        .configure(
            &AuthenticatedIdentity::new(
                UserId::from_uuid(fixture.owner),
                "test",
                "gateway-configure-owner",
                serde_json::json!({}),
                RequestId::new(),
            ),
            ConfigureGatewayRequest {
                gateway_id: fixture.gateway,
                expected_revision_id: first.revision_id,
                parameters: BTreeMap::new(),
                secret_selections: Vec::new(),
            },
        )
        .await
        .expect("second immutable configuration");
    let replay = management
        .configure(
            &owner,
            ConfigureGatewayRequest {
                gateway_id: fixture.gateway,
                expected_revision_id: fixture.revision,
                parameters: BTreeMap::new(),
                secret_selections: Vec::new(),
            },
        )
        .await
        .expect("retry returns original result after later activation");
    assert_eq!(replay.revision_id, first.revision_id);
    assert_ne!(second.revision_id, replay.revision_id);
    let binding_identity = AuthenticatedIdentity::new(
        UserId::from_uuid(fixture.owner),
        "test",
        "gateway-binding-receipt",
        serde_json::json!({}),
        RequestId::new(),
    );
    let binding = management
        .create_mailbox_binding(
            &binding_identity,
            second.revision_id,
            "deliver",
            fixture.mailbox,
            "binding-proof",
        )
        .await
        .expect("create gateway mailbox binding");
    let mut binding_replay_identity = binding_identity.clone();
    binding_replay_identity.request_id = RequestId::new();
    let binding_replay = management
        .create_mailbox_binding(
            &binding_replay_identity,
            second.revision_id,
            "deliver",
            fixture.mailbox,
            "binding-proof",
        )
        .await
        .expect("replay gateway mailbox binding");
    assert_eq!(binding.id, binding_replay.id);
    assert_eq!(binding.grant_id, binding_replay.grant_id);
    assert_eq!(
        binding_identity.idempotency_id,
        binding_replay_identity.idempotency_id
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_events
             WHERE occurrence_id = $1 AND aggregate_type = 'gateway'
               AND aggregate_id = $2",
        )
        .bind(binding_identity.idempotency_id.as_uuid())
        .bind(fixture.gateway)
        .fetch_one(&pool)
        .await
        .expect("gateway binding product event receipt"),
        1
    );
    let revoke_identity = AuthenticatedIdentity::new(
        UserId::from_uuid(fixture.owner),
        "test",
        "gateway-binding-revoke",
        serde_json::json!({}),
        RequestId::new(),
    );
    let revoked = management
        .revoke_mailbox_binding_grant(&revoke_identity, binding.id)
        .await
        .expect("revoke gateway mailbox binding grant");
    let mut revoke_replay_identity = revoke_identity.clone();
    revoke_replay_identity.request_id = RequestId::new();
    let revoke_replay = management
        .revoke_mailbox_binding_grant(&revoke_replay_identity, binding.id)
        .await
        .expect("replay gateway mailbox binding grant revoke");
    assert_eq!(revoked.id, revoke_replay.id);
    assert_eq!(revoked.grant_id, revoke_replay.grant_id);
    assert_eq!(revoked.grant_status, "revoked");
    let outsider = AuthenticatedIdentity::new(
        UserId::from_uuid(Uuid::new_v4()),
        "test",
        "gateway-configure-outsider",
        serde_json::json!({}),
        RequestId::new(),
    );
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Gateway configure outsider')")
        .bind(outsider.user_id.as_uuid())
        .execute(&pool)
        .await
        .expect("configure outsider");
    assert!(matches!(
        management
            .configure(
                &outsider,
                ConfigureGatewayRequest {
                    gateway_id: fixture.gateway,
                    expected_revision_id: second.revision_id,
                    parameters: BTreeMap::new(),
                    secret_selections: Vec::new(),
                },
            )
            .await,
        Err(GatewayConfigureError::Denied)
    ));
}
