use super::{
    EXPECTED_DATABASE_MIGRATION, IsolatedDatabase, fixture::app_config, seed::seed_aged_cleaned_log,
};
use hephaestus_app::HephaestusApp;
use serial_test::serial;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::time::{Duration, Instant};
use tokio::time::{sleep, timeout};
use url::Url;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct RemainingEpoch {
    epoch_count: i64,
    min_fencing_token: Option<i64>,
    max_fencing_token: Option<i64>,
    min_acknowledged_through: Option<i64>,
    min_retained_bytes: Option<i64>,
    min_retained_chunks: Option<i64>,
}

async fn wait_for_retention(pool: &PgPool, project: Uuid) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let row = timeout(
            Duration::from_secs(2),
            sqlx::query(
            "SELECT
                (SELECT count(*) FROM gateway_service_log_chunks WHERE project_id = $1) AS chunks,
                (SELECT count(*) FROM gateway_service_log_epochs WHERE project_id = $1) AS epochs,
                (SELECT retained_bytes FROM gateway_service_log_project_usage WHERE project_id = $1) AS bytes,
                (SELECT retained_chunks FROM gateway_service_log_project_usage WHERE project_id = $1) AS usage_chunks,
                (SELECT retained_epochs FROM gateway_service_log_project_usage WHERE project_id = $1) AS usage_epochs,
                (SELECT count(*)
                   FROM gateway_service_instances instance
                   JOIN gateways gateway ON gateway.id = instance.gateway_id
                  WHERE gateway.project_id = $1 AND instance.state <> 'cleaned') AS live_instances",
            )
            .bind(project)
            .fetch_one(pool),
        )
        .await
        .expect("bounded automatic retention query")
        .expect("query automatic retention result");
        let chunks: i64 = row.get("chunks");
        let epochs: i64 = row.get("epochs");
        let bytes: i64 = row.get("bytes");
        let usage_chunks: i64 = row.get("usage_chunks");
        let usage_epochs: i32 = row.get("usage_epochs");
        let live_instances: i64 = row.get("live_instances");
        if chunks == 0
            && epochs == 1
            && bytes == 0
            && usage_chunks == 0
            && usage_epochs == 1
            && live_instances == 0
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "automatic service-log retention did not complete: chunks={chunks} epochs={epochs} bytes={bytes} usage_chunks={usage_chunks} usage_epochs={usage_epochs} live_instances={live_instances}"
        );
        sleep(Duration::from_millis(100)).await;
    }
}

async fn cleanup_nats(nats_url: &str) {
    let client = async_nats::connect(nats_url)
        .await
        .expect("connect NATS cleanup client");
    let context = async_nats::jetstream::new(client);
    for stream in [
        "HEPH_RUN_COMMANDS",
        "HEPH_RUN_EVENTS",
        "HEPHAESTUS_GIT_EVENTS",
        "HEPHAESTUS_RELEASE_EVENTS",
        "HEPHAESTUS_PRODUCT_EVENTS",
    ] {
        if let Err(error) = context.delete_stream(stream).await {
            assert!(
                error.to_string().contains("stream not found"),
                "delete test NATS stream: {error}"
            );
        }
    }
}

fn require_disposable_nats(nats_url: &str) {
    let parsed = Url::parse(nats_url).expect("parse NATS test URL");
    assert_eq!(parsed.scheme(), "nats", "retention test requires NATS URL");
    assert!(
        matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "::1")),
        "retention test refuses non-loopback NATS URL"
    );
}

const fn retention_supports_migration(version: i64) -> bool {
    version >= 81
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn application_retention_runs_without_gateway_edge_and_closes_worker_pool() {
    if std::env::var("REAL_APP_SERVICE_LOG_MAINTENANCE").as_deref() != Ok("1") {
        eprintln!(
            "SKIPPED application service-log retention: set REAL_APP_SERVICE_LOG_MAINTENANCE=1"
        );
        return;
    }
    assert!(
        retention_supports_migration(EXPECTED_DATABASE_MIGRATION),
        "retention requires migration 81 or newer"
    );
    let parent_database_url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL")
        .expect("HEPHAESTUS_POSTGRES_TEST_URL is required for real retention validation");
    let nats_url = std::env::var("HEPHAESTUS_NATS_TEST_URL")
        .expect("HEPHAESTUS_NATS_TEST_URL is required for real retention validation");
    require_disposable_nats(&nats_url);
    let isolated = IsolatedDatabase::create(&parent_database_url).await;
    let root = tempfile::tempdir().expect("application retention test root");
    let seed_pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&isolated.target_url)
        .await
        .expect("connect isolated seed pool");
    let project = seed_aged_cleaned_log(&seed_pool).await;
    seed_pool.close().await;

    let app = HephaestusApp::build(app_config(
        isolated.target_url.clone(),
        nats_url.clone(),
        root.path(),
    ))
    .await
    .expect("build application with gateway edge disabled")
    .start()
    .await
    .expect("start application with gateway edge disabled");
    let service_log_pool = app.service_log_pool_for_test();
    wait_for_retention(&service_log_pool, project).await;
    let remaining_epoch: RemainingEpoch = sqlx::query_as(
        "SELECT count(*) AS epoch_count,
                min(fencing_token) AS min_fencing_token,
                max(fencing_token) AS max_fencing_token,
                min(acknowledged_through) AS min_acknowledged_through,
                min(retained_bytes) AS min_retained_bytes,
                min(retained_chunks) AS min_retained_chunks
           FROM gateway_service_log_epochs WHERE project_id = $1",
    )
    .bind(project)
    .fetch_one(&service_log_pool)
    .await
    .expect("read post-retention epoch metadata");
    assert_eq!(
        (
            remaining_epoch.epoch_count,
            remaining_epoch.min_fencing_token,
            remaining_epoch.max_fencing_token,
            remaining_epoch.min_acknowledged_through,
            remaining_epoch.min_retained_bytes,
            remaining_epoch.min_retained_chunks,
        ),
        (1, Some(1), Some(1), Some(0), Some(0), Some(0)),
        "empty aged epoch should be GC'd while the payload epoch remains as fresh metadata"
    );
    app.shutdown()
        .await
        .expect("shutdown application after retention");
    assert!(
        service_log_pool.is_closed(),
        "dedicated service-log pool closes"
    );

    cleanup_nats(&nats_url).await;
    let max_version = isolated.max_version;
    isolated.drop().await;
    println!("REAL_APP_SERVICE_LOG_MAINTENANCE=1 max_migration={max_version}");
}
