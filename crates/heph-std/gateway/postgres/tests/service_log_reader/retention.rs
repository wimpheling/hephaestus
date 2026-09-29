//! Retention snapshot checks for the service log reader.

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
pub async fn verify_retention(
    admin: &sqlx::PgPool,
    reader: &PostgresGatewayServiceLogReader,
    fixture: &Fixture,
    scope: GatewayServiceLogReadScope,
    owner: &AuthenticatedIdentity,
) {
    let mut retention_audit_lock = admin.begin().await.expect("begin retention barrier");
    sqlx::query("LOCK TABLE authorization_audit_events IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *retention_audit_lock)
        .await
        .expect("hold retention audit barrier");
    let retention_holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *retention_audit_lock)
        .await
        .expect("read retention barrier backend");
    let retention_reader = reader.clone();
    let retention_identity = identity(fixture.owner, "retention-concurrent");
    let retention_request = GatewayServiceLogReadRequest::new(
        scope,
        100,
        Some(GatewayServiceLogReadCursor::new(scope, 119).expect("retention cursor")),
    )
    .expect("retention page request");
    let retention_reader_task = spawn(async move {
        retention_reader
            .get_page(&retention_identity, retention_request)
            .await
    });
    let mut observed_retention_barrier = false;
    for _ in 0..100 {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM pg_stat_activity AS waiting
                  WHERE waiting.application_name = 'heph-service-log-reader-page'
                    AND waiting.wait_event_type = 'Lock'
                    AND $1 = ANY(pg_blocking_pids(waiting.pid))
             )",
        )
        .bind(retention_holder_pid)
        .fetch_one(admin)
        .await
        .expect("observe retention reader barrier");
        if waiting {
            observed_retention_barrier = true;
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }
    assert!(
        observed_retention_barrier,
        "retention reader did not reach audit barrier"
    );
    let mut retention_writer = admin.begin().await.expect("begin retention mutation");
    sqlx::query(
        "DELETE FROM gateway_service_log_chunks
          WHERE instance_id = $1
            AND gateway_id = $2
            AND revision_id = $3
            AND project_id = $4
            AND fencing_token = 1
            AND sequence = 120",
    )
    .bind(fixture.instance)
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(fixture.project)
    .execute(&mut *retention_writer)
    .await
    .expect("delete retained chunk");
    sqlx::query(
        "INSERT INTO gateway_service_log_chunks
            (instance_id, gateway_id, revision_id, project_id, fencing_token,
             sequence, stream, observed_at, bytes)
         VALUES ($1, $2, $3, $4, 1, 122, 'stderr', now(), $5)",
    )
    .bind(fixture.instance)
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(fixture.project)
    .bind(b"retention-replacement".as_slice())
    .execute(&mut *retention_writer)
    .await
    .expect("append replacement chunk");
    sqlx::query(
        "UPDATE gateway_service_log_epochs
            SET acknowledged_through = 122,
                retained_bytes = 589_992,
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
    .execute(&mut *retention_writer)
    .await
    .expect("update retention metadata");
    retention_writer
        .commit()
        .await
        .expect("commit retention mutation");
    retention_audit_lock
        .commit()
        .await
        .expect("release retention barrier");
    let retained_snapshot = retention_reader_task
        .await
        .expect("join retention reader")
        .expect("read retention snapshot");
    assert_eq!(
        retained_snapshot
            .records
            .iter()
            .map(|record| record.sequence)
            .collect::<Vec<_>>(),
        vec![120, 121]
    );
    assert_eq!(retained_snapshot.metadata.acknowledged_through, Some(121));
    assert_eq!(retained_snapshot.metadata.retained_bytes, 589_972);
    assert_eq!(retained_snapshot.metadata.retained_chunks, 112);
    assert_eq!(retained_snapshot.records[0].bytes, b"x");
    assert_eq!(
        retained_snapshot.records[1].bytes,
        b"committed-after-snapshot"
    );
    let retained_next = reader
        .get_page(
            owner,
            GatewayServiceLogReadRequest::new(
                scope,
                100,
                Some(GatewayServiceLogReadCursor::new(scope, 119).expect("retention next cursor")),
            )
            .expect("valid retention next request"),
        )
        .await
        .expect("read post-retention page");
    assert_eq!(
        retained_next
            .records
            .iter()
            .map(|record| record.sequence)
            .collect::<Vec<_>>(),
        vec![121, 122]
    );
    assert_eq!(retained_next.metadata.acknowledged_through, Some(122));
    assert_eq!(retained_next.metadata.retained_bytes, 589_992);
    assert_eq!(retained_next.metadata.retained_chunks, 112);
    assert_eq!(retained_next.records[0].bytes, b"committed-after-snapshot");
    assert_eq!(retained_next.records[1].bytes, b"retention-replacement");

    let max_cursor_page = reader
        .get_page(
            owner,
            GatewayServiceLogReadRequest::new(
                scope,
                100,
                Some(
                    GatewayServiceLogReadCursor::new(scope, i64::MAX as u64)
                        .expect("maximum cursor"),
                ),
            )
            .expect("valid maximum cursor request"),
        )
        .await
        .expect("maximum cursor page");
    assert!(max_cursor_page.records.is_empty());
    assert!(max_cursor_page.next_after.is_none());
}
