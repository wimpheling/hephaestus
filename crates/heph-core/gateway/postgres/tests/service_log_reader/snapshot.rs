//! Snapshot consistency checks for the service log reader.

use super::support::{Fixture, identity};
use gateway_domain::{
    GatewayServiceLogReadCursor, GatewayServiceLogReadRequest, GatewayServiceLogReadScope,
};
use gateway_postgres::PostgresGatewayServiceLogReader;
use identity_domain::AuthenticatedIdentity;
use std::time::Duration;
use tokio::{spawn, time::sleep};

// The phase keeps related authorization/snapshot assertions together.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn verify_snapshot(
    admin: &sqlx::PgPool,
    reader: &PostgresGatewayServiceLogReader,
    fixture: &Fixture,
    scope: GatewayServiceLogReadScope,
    owner: &AuthenticatedIdentity,
) {
    let mut audit_lock = admin.begin().await.expect("begin snapshot barrier");
    sqlx::query("LOCK TABLE authorization_audit_events IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *audit_lock)
        .await
        .expect("hold audit snapshot barrier");
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *audit_lock)
        .await
        .expect("read snapshot barrier backend");
    let concurrent_reader = reader.clone();
    let concurrent_identity = identity(fixture.owner, "concurrent");
    let concurrent_request = GatewayServiceLogReadRequest::new(
        scope,
        100,
        Some(GatewayServiceLogReadCursor::new(scope, 119).expect("concurrent cursor")),
    )
    .expect("concurrent page request");
    let reader_task = spawn(async move {
        concurrent_reader
            .get_page(&concurrent_identity, concurrent_request)
            .await
    });
    let mut observed_snapshot_barrier = false;
    for _ in 0..100 {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM pg_stat_activity AS waiting
                  WHERE waiting.application_name = 'heph-service-log-reader-page'
                    AND waiting.wait_event_type = 'Lock'
                    AND $1 = ANY(pg_blocking_pids(waiting.pid))
             )",
        )
        .bind(holder_pid)
        .fetch_one(admin)
        .await
        .expect("observe reader snapshot barrier");
        if waiting {
            observed_snapshot_barrier = true;
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }
    assert!(
        observed_snapshot_barrier,
        "reader did not reach metadata lock barrier"
    );
    let mut writer = admin.begin().await.expect("begin concurrent append");
    sqlx::query(
        "INSERT INTO gateway_service_log_chunks
            (instance_id, gateway_id, revision_id, project_id, fencing_token,
             sequence, stream, observed_at, bytes)
         VALUES ($1, $2, $3, $4, 1, 121, 'stdout', now(), $5)",
    )
    .bind(fixture.instance)
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(fixture.project)
    .bind(b"committed-after-snapshot".as_slice())
    .execute(&mut *writer)
    .await
    .expect("append after reader snapshot");
    sqlx::query(
        "UPDATE gateway_service_log_epochs
            SET acknowledged_through = 121,
                retained_bytes = 589_972,
                retained_chunks = 112
          WHERE instance_id = $1
            AND gateway_id = $2
            AND revision_id = $3
            AND project_id = $4
            AND fencing_token = 1",
    )
    .bind(fixture.instance)
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(fixture.project)
    .execute(&mut *writer)
    .await
    .expect("update metadata with append");
    writer
        .commit()
        .await
        .expect("commit append snapshot mutation");
    audit_lock.commit().await.expect("release snapshot barrier");
    let snapshot_page = reader_task
        .await
        .expect("join snapshot reader")
        .expect("read coherent snapshot page");
    assert_eq!(
        snapshot_page
            .records
            .iter()
            .map(|record| record.sequence)
            .collect::<Vec<_>>(),
        vec![120]
    );
    assert_eq!(snapshot_page.records[0].bytes, b"x");
    assert_eq!(snapshot_page.metadata.acknowledged_through, Some(120));
    assert_eq!(snapshot_page.metadata.retained_bytes, 589_948);
    assert_eq!(snapshot_page.metadata.retained_chunks, 111);
    let next_page = reader
        .get_page(
            owner,
            GatewayServiceLogReadRequest::new(
                scope,
                100,
                Some(GatewayServiceLogReadCursor::new(scope, 119).expect("next cursor")),
            )
            .expect("valid post-commit page request"),
        )
        .await
        .expect("read post-commit page");
    assert_eq!(
        next_page
            .records
            .iter()
            .map(|record| record.sequence)
            .collect::<Vec<_>>(),
        vec![120, 121]
    );
    assert_eq!(next_page.records[1].bytes, b"committed-after-snapshot");
    assert_eq!(next_page.metadata.acknowledged_through, Some(121));
    assert_eq!(next_page.metadata.retained_bytes, 589_972);
    assert_eq!(next_page.metadata.retained_chunks, 112);
}
