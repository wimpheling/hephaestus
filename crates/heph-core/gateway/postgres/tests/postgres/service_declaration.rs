/// service declaration scenario.
use super::service_declaration_followup;
use super::support::seed_fixture;
use authz_postgres::PostgresMelangeAuthorizer;
use forge_domain::{ProjectId, RepositoryId};
use gateway_postgres::{
    InstallGatewayManifest, PostgresGatewayInstaller, PostgresGatewayManagement,
};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use release_domain::ReleaseId;
use serial_test::serial;
use sha2::Digest;
use sha2::Sha256;
use sqlx::postgres::PgPoolOptions;
use std::{env, sync::Arc};
use uuid::Uuid;

#[tokio::test]
#[serial]
async fn gateway_service_declaration_round_trips_and_rejects_invalid_shapes() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&database_url)
        .await
        .expect("connect gateway service PostgreSQL");
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .expect("apply gateway service migrations");
    let fixture = seed_fixture(&pool).await;

    let stateless: (
        String,
        Option<i32>,
        Option<String>,
        Option<String>,
        String,
        Vec<u8>,
    ) = sqlx::query_as(
        "SELECT handler_contract, service_loopback_port, service_readiness_path,
                    service_health_path, service_log_capture_mode, normalized_hash
             FROM gateway_revisions WHERE id = $1",
    )
    .bind(fixture.revision)
    .fetch_one(&pool)
    .await
    .expect("load legacy stateless revision");
    assert_eq!(stateless.0, "http.v1");
    assert_eq!(stateless.1, None);
    assert_eq!(stateless.2, None);
    assert_eq!(stateless.3, None);
    assert_eq!(stateless.4, "disabled");
    assert_eq!(stateless.5, vec![9_u8; 32]);

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
    let installer =
        PostgresGatewayInstaller::new(app_pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let owner = AuthenticatedIdentity::new(
        UserId::from_uuid(fixture.owner),
        "test",
        "gateway-service-owner",
        serde_json::json!({}),
        RequestId::new(),
    );
    let manifest = br#"
version = 1

[[gateways]]
name = "service-installed"
agent_name = "gateway-mailbox"
handler_contract = "http.service.v1"
exposure = "public"

[gateways.service]
loopback_port = 18080
readiness_path = "/ready"
health_path = "/health"
log_capture_mode = "application"

[[gateways.routes]]
path = "/service"
methods = ["GET"]
"#;
    let installed = installer
        .install(
            &owner,
            InstallGatewayManifest {
                project_id: ProjectId::from_uuid(fixture.project),
                repository_id: RepositoryId::from_uuid(fixture.repository),
                release_id: Some(ReleaseId::from_uuid(fixture.release)),
                manifest: manifest.to_vec(),
            },
        )
        .await
        .expect("install service declaration");
    let initial_state: (Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        "SELECT active_revision_id, desired_service_revision_id
           FROM gateways WHERE id = $1",
    )
    .bind(installed.gateways[0].gateway_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("load initial service activation state");
    assert_eq!(initial_state.0, None);
    assert_eq!(
        initial_state.1,
        Some(installed.gateways[0].revision_id.as_uuid())
    );
    let initial_install_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
           WHERE occurrence_id = $1 AND aggregate_type = 'gateway'
             AND scope_kind = 'project' AND actor_id = $2",
    )
    .bind(owner.idempotency_id.as_uuid())
    .bind(fixture.owner)
    .fetch_one(&pool)
    .await
    .expect("count initial service installation receipt");
    assert_eq!(
        initial_install_events, 2,
        "initial service installation preserves INSERT and desired-assignment events"
    );
    let reinstall_identity = AuthenticatedIdentity::new(
        UserId::from_uuid(fixture.owner),
        "test",
        "gateway-service-reinstall",
        serde_json::json!({}),
        RequestId::new(),
    );
    let reinstalled = installer
        .install(
            &reinstall_identity,
            InstallGatewayManifest {
                project_id: ProjectId::from_uuid(fixture.project),
                repository_id: RepositoryId::from_uuid(fixture.repository),
                release_id: Some(ReleaseId::from_uuid(fixture.release)),
                manifest: manifest.to_vec(),
            },
        )
        .await
        .expect("reinstall existing service declaration");
    assert_eq!(reinstalled, installed);
    let reinstall_receipt: (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE actor_id = $2)
           FROM application_events
           WHERE occurrence_id = $1 AND aggregate_type = 'gateway'
             AND scope_kind = 'project'",
    )
    .bind(reinstall_identity.idempotency_id.as_uuid())
    .bind(fixture.owner)
    .fetch_one(&pool)
    .await
    .expect("load service reinstall receipt");
    assert_eq!(
        reinstall_receipt,
        (1, 1),
        "existing service reinstall emits exactly one actor-scoped gateway receipt"
    );
    let stateless_state: (Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        "SELECT active_revision_id, desired_service_revision_id
           FROM gateways WHERE id = $1",
    )
    .bind(fixture.gateway)
    .fetch_one(&pool)
    .await
    .expect("load stateless activation state");
    assert_eq!(stateless_state, (Some(fixture.revision), None));
    let persisted: (String, Option<i32>, Option<String>, Option<String>, String) = sqlx::query_as(
        "SELECT handler_contract, service_loopback_port, service_readiness_path,
                service_health_path, service_log_capture_mode
         FROM gateway_revisions WHERE id = $1",
    )
    .bind(installed.gateways[0].revision_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("load persisted service declaration");
    assert_eq!(
        persisted,
        (
            "http.service.v1".to_owned(),
            Some(18080),
            Some("/ready".to_owned()),
            Some("/health".to_owned()),
            "application".to_owned(),
        )
    );

    // Use the harness owner connection for the internal projection proof;
    // production management calls use their established role boundary.
    let management =
        PostgresGatewayManagement::new(pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let (_, revisions) = management
        .get(&owner, installed.gateways[0].gateway_id.as_uuid())
        .await
        .expect("project service revision");
    let service = revisions
        .iter()
        .find(|revision| revision.id == installed.gateways[0].revision_id.as_uuid())
        .and_then(|revision| revision.service.clone())
        .expect("typed service projection");
    assert_eq!(service.loopback_port, 18080);
    assert_eq!(service.readiness_path.as_str(), "/ready");
    assert_eq!(service.health_path.as_str(), "/health");
    assert_eq!(
        service.log_capture_mode,
        gateway_domain::ServiceLogCaptureMode::Application
    );

    service_declaration_followup::exercise_service_configuration(
        &pool,
        &management,
        &installer,
        &owner,
        &fixture,
        installed.gateways[0].gateway_id.as_uuid(),
        installed.gateways[0].revision_id.as_uuid(),
        &persisted,
    )
    .await;
    let oversized_path = format!("/{}", "é".repeat(256));
    for (port, readiness, health) in [
        (
            Some(80_i32),
            Some("/ready".to_owned()),
            Some("/health".to_owned()),
        ),
        (
            Some(18080_i32),
            Some("ready".to_owned()),
            Some("/health".to_owned()),
        ),
        (
            Some(18080_i32),
            Some("/ready%20".to_owned()),
            Some("/health".to_owned()),
        ),
        (Some(18080_i32), None, Some("/health".to_owned())),
        (
            Some(18080_i32),
            Some(oversized_path),
            Some("/health".to_owned()),
        ),
    ] {
        let mut transaction = pool.begin().await.expect("begin invalid shape transaction");
        let result = sqlx::query(
            "INSERT INTO gateway_revisions
                (id, gateway_id, project_id, repository_id, handler_contract,
                 exposure, parameters, service_loopback_port, service_readiness_path,
                 service_health_path, normalized_hash, created_by)
             VALUES ($1, $2, $3, $4, 'http.service.v1', 'public', '{}', $5, $6, $7, $8, $9)",
        )
        .bind(Uuid::new_v4())
        .bind(fixture.gateway)
        .bind(fixture.project)
        .bind(fixture.repository)
        .bind(port)
        .bind(readiness)
        .bind(health)
        .bind(Sha256::digest(Uuid::new_v4().as_bytes()).as_slice())
        .bind(fixture.owner)
        .execute(&mut *transaction)
        .await;
        assert!(result.is_err(), "invalid service shape must be rejected");
        transaction
            .rollback()
            .await
            .expect("rollback invalid shape transaction");
    }

    for (handler_contract, log_capture_mode, port, readiness, health) in [
        (
            "http.service.v1",
            "future",
            Some(18080_i32),
            Some("/ready".to_owned()),
            Some("/health".to_owned()),
        ),
        ("http.v1", "application", None, None, None),
    ] {
        let mut transaction = pool.begin().await.expect("begin invalid mode transaction");
        let result = sqlx::query(
            "INSERT INTO gateway_revisions
                (id, gateway_id, project_id, repository_id, handler_contract,
                 exposure, parameters, service_loopback_port, service_readiness_path,
                 service_health_path, service_log_capture_mode, normalized_hash, created_by)
             VALUES ($1, $2, $3, $4, $5, 'public', '{}', $6, $7, $8, $9, $10, $11)",
        )
        .bind(Uuid::new_v4())
        .bind(fixture.gateway)
        .bind(fixture.project)
        .bind(fixture.repository)
        .bind(handler_contract)
        .bind(port)
        .bind(readiness)
        .bind(health)
        .bind(log_capture_mode)
        .bind(Sha256::digest(Uuid::new_v4().as_bytes()).as_slice())
        .bind(fixture.owner)
        .execute(&mut *transaction)
        .await;
        assert!(
            result.is_err(),
            "invalid log capture mode shape must be rejected"
        );
        transaction
            .rollback()
            .await
            .expect("rollback invalid mode transaction");
    }

    let invalid_desired =
        sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
            .bind(installed.gateways[0].gateway_id.as_uuid())
            .bind(fixture.revision)
            .execute(&pool)
            .await;
    assert!(
        invalid_desired.is_err(),
        "desired state must reject a stateless revision"
    );
}
