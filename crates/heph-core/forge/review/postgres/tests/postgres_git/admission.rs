use control_plane_postgres::run::{
    ControlKind as AdmissionKind, ControlTarget, RequestControl, RunApplication,
};
use forge_service::GitStorage;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use runtime_types::RunId;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tempfile::TempDir;
use uuid::Uuid;

use super::support::{pool, seed};

#[tokio::test]
#[serial]
async fn app_role_admission_rejects_unsupported_retry_without_disclosing_to_unauthorized_actor() {
    let Some(pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&pool)
        .await
        .expect("apply migrations");
    let temporary = TempDir::new().expect("temporary fixture");
    let storage = Arc::new(
        GitStorage::initialize(temporary.path().join("repositories"))
            .await
            .expect("Git storage"),
    );
    let fixture = seed(&pool, &storage, &temporary).await;
    let orphan_run_id = RunId::new();
    sqlx::query(
        "INSERT INTO runs
         (id, instance_id, instance_revision_id, release_id, release_agent_id,
          attachment_id, run_kind, command_id, state, requires_state,
          created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, 'normal', $7, 'cleaned_up', false,
                 now(), now())",
    )
    .bind(orphan_run_id.as_uuid())
    .bind(fixture.instance_id)
    .bind(fixture.instance_revision_id)
    .bind(fixture.release_id)
    .bind(fixture.release_agent_id)
    .bind(fixture.attachment_id)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("orphan run");
    let database_url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL")
        .expect("test URL is present when pool exists");
    let app_pool = PgPoolOptions::new()
        .max_connections(2)
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
    let app_user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&app_pool)
        .await
        .expect("application session role");
    assert_eq!(app_user, "hephaestus_app");
    let application = RunApplication::new(app_pool, temporary.path().join("artifacts"));
    let identity = AuthenticatedIdentity::new(
        fixture.actor_id,
        "https://retry-regression.example",
        "reviewer",
        serde_json::json!({"email_verified": true}),
        RequestId::new(),
    );
    let request = RequestControl {
        kind: AdmissionKind::Retry,
        repository_id: fixture.repository_id.as_uuid(),
        target: ControlTarget::Run(orphan_run_id.as_uuid()),
        reason: String::from("fixture"),
    };
    let admitted = application
        .request_control(&identity, request)
        .await
        .expect("authorized unsupported retry admission");
    assert_eq!(admitted.state, "failed");
    let replay = application
        .request_control(
            &identity,
            RequestControl {
                kind: AdmissionKind::Retry,
                repository_id: fixture.repository_id.as_uuid(),
                target: ControlTarget::Run(orphan_run_id.as_uuid()),
                reason: String::from("fixture"),
            },
        )
        .await
        .expect("replay unsupported retry admission");
    assert_eq!(replay.id, admitted.id);
    assert_eq!(replay.state, "failed");
    let (state, diagnostics): (String, serde_json::Value) =
        sqlx::query_as("SELECT state, diagnostics FROM control_requests WHERE id = $1")
            .bind(admitted.id)
            .fetch_one(&pool)
            .await
            .expect("admission control row");
    assert_eq!(state, "failed");
    assert_eq!(
        diagnostics,
        serde_json::json!([{ "code": "retry_unsupported" }])
    );
    let retry_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_requests WHERE retry_of_run_id = $1")
            .bind(orphan_run_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("admission retry count");
    assert_eq!(retry_count, 0);
    let failed_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE scope_kind = 'run' AND scope_id = $1
           AND aggregate_type = 'run' AND event_type = 'run.changed'
           AND safe_state = 'failed' AND actor_id = $2",
    )
    .bind(orphan_run_id.as_uuid())
    .bind(fixture.actor_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("admission failed event count");
    assert!(failed_events >= 1);

    let outsider_id = UserId::new();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Unauthorized')")
        .bind(outsider_id.as_uuid())
        .execute(&pool)
        .await
        .expect("outsider");
    let outsider_identity = AuthenticatedIdentity::new(
        outsider_id,
        "https://retry-regression.example",
        "outsider",
        serde_json::json!({"email_verified": true}),
        RequestId::new(),
    );
    let unauthorized = application
        .request_control(
            &outsider_identity,
            RequestControl {
                kind: AdmissionKind::Retry,
                repository_id: fixture.repository_id.as_uuid(),
                target: ControlTarget::Run(orphan_run_id.as_uuid()),
                reason: String::from("fixture"),
            },
        )
        .await;
    assert!(unauthorized.is_err());
    let outsider_controls: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM control_requests WHERE actor_id = $1 AND run_id = $2",
    )
    .bind(outsider_id.as_uuid())
    .bind(orphan_run_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("outsider control count");
    assert_eq!(outsider_controls, 0);
}
