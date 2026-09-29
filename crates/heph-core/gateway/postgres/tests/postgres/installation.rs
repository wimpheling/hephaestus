/// installation scenario.
use super::support::{
    assert_reconfigure_after_reinstallation_creates_fresh_authority, seed_fixture,
};
use authz_postgres::PostgresMelangeAuthorizer;
use forge_domain::{ProjectId, RepositoryId};
use gateway_postgres::{InstallGatewayManifest, PostgresGatewayInstaller};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use release_domain::ReleaseId;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::{env, sync::Arc};
use uuid::Uuid;

#[tokio::test]
#[serial]
async fn gateway_release_installation_is_authorized_published_and_idempotent() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&database_url)
        .await
        .expect("connect gateway installation PostgreSQL");
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .expect("apply gateway installation migrations");
    let fixture = seed_fixture(&pool).await;
    // Exercise the same restricted role path as the application connection.
    // Role membership is deployment-owned; this test only changes the
    // session role and never performs schema-changing DDL.
    let app_pool = PgPoolOptions::new()
        .max_connections(6)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect as hephaestus application role");
    let current_user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&app_pool)
        .await
        .expect("application session role");
    assert_eq!(current_user, "hephaestus_app");
    let installer = PostgresGatewayInstaller::new(app_pool, Arc::new(PostgresMelangeAuthorizer));
    let owner = AuthenticatedIdentity::new(
        UserId::from_uuid(fixture.owner),
        "test",
        "gateway-owner",
        serde_json::json!({}),
        RequestId::new(),
    );
    let release_id = ReleaseId::from_uuid(fixture.release);
    let source = installer
        .published_release(&owner, release_id)
        .await
        .expect("owner resolves published release");
    assert_eq!(
        source.repository_id,
        RepositoryId::from_uuid(fixture.repository)
    );
    let manifest = br#"
version = 1

[[gateways]]
name = "installed"
agent_name = "gateway-mailbox"
handler_contract = "http.v1"
exposure = "public"

[[gateways.routes]]
path = "/installed"
methods = ["POST"]
"#;
    let command = || InstallGatewayManifest {
        project_id: ProjectId::from_uuid(fixture.project),
        repository_id: RepositoryId::from_uuid(fixture.repository),
        release_id: Some(release_id),
        manifest: manifest.to_vec(),
    };
    let first = installer.install(&owner, command()).await.expect("install");
    let (events, pending_outbox): (i64, i64) = sqlx::query_as(
        "SELECT
             (SELECT count(*) FROM application_events
               WHERE aggregate_type = 'gateway' AND aggregate_id = $1
                 AND occurrence_id = $2),
             (SELECT count(*) FROM product_event_outbox outbox
               JOIN application_events event ON event.id = outbox.event_id
               WHERE event.aggregate_type = 'gateway' AND event.aggregate_id = $1
                 AND event.occurrence_id = $2)",
    )
    .bind(first.gateways[0].gateway_id.as_uuid())
    .bind(owner.idempotency_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("gateway installation event and outbox");
    assert!(
        events > 0,
        "installation must commit a gateway product event"
    );
    assert_eq!(events, pending_outbox);
    let second = installer
        .install(&owner, command())
        .await
        .expect("repeat install");
    assert_eq!(
        first, second,
        "reinstalling one release must select its existing immutable revision"
    );

    let second_release = Uuid::new_v4();
    let second_build = Uuid::new_v4();
    let second_agent = Uuid::new_v4();
    let hash = [9_u8; 32];
    sqlx::query("INSERT INTO build_requests (id, repository_id, source_commit, source_ref, build_definition_hash, state, completed_at) VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', now())")
        .bind(second_build)
        .bind(fixture.repository)
        .bind("b".repeat(40))
        .bind(hash.as_slice())
        .execute(&pool)
        .await
        .expect("second build");
    sqlx::query("INSERT INTO releases (id, repository_id, version, source_commit, source_ref, build_request_id, build_definition_hash, configuration, configuration_hash, manifest_hash, state, published_at) VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}', $7, $8, 'published', now())")
        .bind(second_release)
        .bind(fixture.repository)
        .bind(format!("gateway-install-{second_release}"))
        .bind("b".repeat(40))
        .bind(second_build)
        .bind(hash.as_slice())
        .bind(hash.as_slice())
        .bind(hash.as_slice())
        .execute(&pool)
        .await
        .expect("second release");
    sqlx::query("INSERT INTO release_agents (id, release_id, family_id, agent_key, display_name, runtime_contract, runtime_contract_hash, parameter_schema, secret_slot_schema, requires_state) VALUES ($1, $2, $3, 'gateway-mailbox', 'Gateway mailbox', '{}', $4, '[]', '[]', false)")
        .bind(second_agent)
        .bind(second_release)
        .bind(fixture.family)
        .bind(hash.as_slice())
        .execute(&pool)
        .await
        .expect("second release agent");
    let second_owner = AuthenticatedIdentity::new(
        UserId::from_uuid(fixture.owner),
        "test",
        "gateway-owner-second-request",
        serde_json::json!({}),
        RequestId::new(),
    );
    let second_install = installer
        .install(
            &second_owner,
            InstallGatewayManifest {
                project_id: ProjectId::from_uuid(fixture.project),
                repository_id: RepositoryId::from_uuid(fixture.repository),
                release_id: Some(ReleaseId::from_uuid(second_release)),
                manifest: manifest.to_vec(),
            },
        )
        .await
        .expect("install second release");
    assert_ne!(
        first.gateways[0].revision_id, second_install.gateways[0].revision_id,
        "the same declaration from a new release must select a new immutable revision"
    );
    assert_eq!(
        installer
            .install(&owner, command())
            .await
            .expect("replay key1"),
        first,
        "retrying key1 must return its original revision after key2 activates a newer release"
    );
    let fresh_key = owner.clone().with_idempotency_id(RequestId::new());
    let fresh_install = installer
        .install(&fresh_key, command())
        .await
        .expect("fresh key same release is a successful no-op install");
    assert_eq!(fresh_install, first);
    assert_reconfigure_after_reinstallation_creates_fresh_authority(
        &pool,
        &installer,
        &owner,
        command(),
        first.gateways[0],
    )
    .await;
    assert!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_events
             WHERE occurrence_id = $1 AND aggregate_type = 'gateway'",
        )
        .bind(fresh_key.idempotency_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("fresh no-op installation receipt event")
            > 0
    );
    assert!(matches!(
        installer
            .install(
                &owner,
                InstallGatewayManifest {
                    project_id: ProjectId::from_uuid(fixture.project),
                    repository_id: RepositoryId::from_uuid(fixture.repository),
                    release_id: Some(ReleaseId::from_uuid(second_release)),
                    manifest: manifest.to_vec(),
                },
            )
            .await,
        Err(gateway_postgres::GatewayInstallError::Conflict)
    ));
    let denied_user = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Gateway Denied Actor')")
        .bind(denied_user)
        .execute(&pool)
        .await
        .expect("denied user");
    let unauthorized = AuthenticatedIdentity::new(
        UserId::from_uuid(denied_user),
        "test",
        "gateway-denied",
        serde_json::json!({}),
        RequestId::new(),
    );
    assert!(matches!(
        installer.install(&unauthorized, command()).await,
        Err(gateway_postgres::GatewayInstallError::AuthorizationDenied)
    ));
    assert!(matches!(
        installer
            .install(
                &owner,
                InstallGatewayManifest {
                    project_id: ProjectId::from_uuid(fixture.project),
                    repository_id: RepositoryId::from_uuid(fixture.repository),
                    release_id: Some(release_id),
                    manifest: b"version = 1\n".to_vec(),
                },
            )
            .await,
        Err(gateway_postgres::GatewayInstallError::InvalidManifest { .. })
    ));

    let draft_release = Uuid::new_v4();
    sqlx::query("INSERT INTO releases (id, repository_id, version, source_commit, source_ref, build_request_id, build_definition_hash, configuration, configuration_hash, manifest_hash, state) VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}', $7, $8, 'draft')")
        .bind(draft_release)
        .bind(fixture.repository)
        .bind(format!("gateway-draft-{draft_release}"))
        .bind("c".repeat(40))
        .bind(second_build)
        .bind(hash.as_slice())
        .bind(hash.as_slice())
        .bind(hash.as_slice())
        .execute(&pool)
        .await
        .expect("draft release");
    assert!(matches!(
        installer
            .published_release(&owner, ReleaseId::from_uuid(draft_release))
            .await,
        Err(gateway_postgres::GatewayInstallError::Unavailable)
    ));
}
