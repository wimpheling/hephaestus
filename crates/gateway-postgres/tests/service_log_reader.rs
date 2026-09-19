//! Real `PostgreSQL` coverage for authorized service-log epoch metadata reads.

use gateway_edge::{
    GatewayServiceLogReadCursor, GatewayServiceLogReadMetadata, GatewayServiceLogReadRequest,
    GatewayServiceLogReadScope, GatewayServiceOwnership,
};
use gateway_postgres::{GatewayServiceLogReaderError, PostgresGatewayServiceLogReader};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::{env, sync::Arc, time::Duration};
use tokio::{task::spawn, time::sleep};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn authorized_epoch_metadata_is_scoped_and_audited() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("SKIPPED service log reader: HEPHAESTUS_POSTGRES_TEST_URL is unset");
        return;
    };
    let admin = PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    sqlx::migrate!("../../migrations")
        .run(&admin)
        .await
        .expect("apply gateway migrations");
    let max_version: i64 = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(version) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&admin)
    .await
    .expect("read migration marker")
    .expect("migrations exist");
    assert!(max_version >= 81);
    println!("REAL_POSTGRES_LOG_READER=1 max_migration={max_version}");

    let fixture = seed_fixture(&admin).await;
    let app = PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SET application_name = 'heph-service-log-reader-page'")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect application role");
    let current_user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&app)
        .await
        .expect("read application role");
    assert_eq!(current_user, "hephaestus_app");
    for query in [
        "SELECT repository_id FROM gateways LIMIT 1",
        "SELECT project_id FROM gateway_revisions LIMIT 1",
    ] {
        let error = sqlx::query(query)
            .fetch_optional(&app)
            .await
            .expect_err("application role must not read ungranted gateway columns");
        let database_error = error
            .as_database_error()
            .expect("permission failure must be a database error");
        assert_eq!(database_error.code().as_deref(), Some("42501"));
    }

    let reader = PostgresGatewayServiceLogReader::new(
        app,
        Arc::new(authz_postgres::PostgresMelangeAuthorizer),
    );
    let scope = GatewayServiceLogReadScope::new(
        fixture.project,
        fixture.gateway,
        fixture.revision,
        fixture.instance,
        1,
    )
    .expect("valid fixture scope");
    let owner = identity(fixture.owner, "owner");
    let member = identity(fixture.member, "member");
    let outsider = identity(fixture.outsider, "outsider");

    let empty = reader
        .get_epoch_metadata(&owner, scope)
        .await
        .expect("known instance without epoch");
    assert_eq!(empty, GatewayServiceLogReadMetadata::default());
    let empty_page = reader
        .get_page(
            &owner,
            GatewayServiceLogReadRequest::new(scope, 100, None).expect("valid empty page request"),
        )
        .await
        .expect("known instance empty page");
    assert!(empty_page.records.is_empty());
    assert!(!empty_page.history_incomplete);

    sqlx::query(
        "INSERT INTO gateway_service_log_epochs
            (instance_id, gateway_id, revision_id, project_id, fencing_token,
             acknowledged_through, retained_bytes, retained_chunks,
             producer_dropped_chunks, producer_dropped_bytes,
             provider_lagged_events, storage_dropped_chunks, storage_dropped_bytes,
             evicted_chunks, evicted_bytes)
         VALUES ($1, $2, $3, $4, 1, 120, 589_948, 111, 2, 3, 4, 5, 6, 7, 8)",
    )
    .bind(fixture.instance)
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(fixture.project)
    .execute(&admin)
    .await
    .expect("insert log epoch metadata");
    sqlx::query(
        "INSERT INTO gateway_service_log_chunks
            (instance_id, gateway_id, revision_id, project_id, fencing_token,
             sequence, stream, observed_at, bytes)
         VALUES ($1, $2, $3, $4, 1, 10, 'stdout', now(), $5)",
    )
    .bind(fixture.instance)
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(fixture.project)
    .bind(b"reader-metadata-payload".as_slice())
    .execute(&admin)
    .await
    .expect("insert log payload fixture");
    for sequence in 11_i64..20 {
        sqlx::query(
            "INSERT INTO gateway_service_log_chunks
                (instance_id, gateway_id, revision_id, project_id, fencing_token,
                 sequence, stream, observed_at, bytes)
             VALUES ($1, $2, $3, $4, 1, $5, 'stdout', now(), $6)",
        )
        .bind(fixture.instance)
        .bind(fixture.gateway)
        .bind(fixture.revision)
        .bind(fixture.project)
        .bind(sequence)
        .bind(vec![b'a'; 65_536])
        .execute(&admin)
        .await
        .expect("insert byte-budget payload fixture");
    }
    for sequence in 20_i64..121 {
        sqlx::query(
            "INSERT INTO gateway_service_log_chunks
                (instance_id, gateway_id, revision_id, project_id, fencing_token,
                 sequence, stream, observed_at, bytes)
             VALUES ($1, $2, $3, $4, 1, $5, 'stderr', now(), $6)",
        )
        .bind(fixture.instance)
        .bind(fixture.gateway)
        .bind(fixture.revision)
        .bind(fixture.project)
        .bind(sequence)
        .bind(b"x".as_slice())
        .execute(&admin)
        .await
        .expect("insert record-cap payload fixture");
    }

    sqlx::query(
        "UPDATE gateway_service_instances
            SET owner_uuid = $2, fencing_token = 2, state = 'stopping',
                lease_expires_at = now() + interval '30 minutes', heartbeat_at = now()
          WHERE id = $1",
    )
    .bind(fixture.instance)
    .bind(Uuid::new_v4())
    .execute(&admin)
    .await
    .expect("advance service instance fence");

    let observed = reader
        .get_epoch_metadata(&owner, scope)
        .await
        .expect("authorized owner metadata");
    assert!(observed.epoch_present);
    assert_eq!(observed.acknowledged_through, Some(120));
    assert_eq!(observed.retained_bytes, 589_948);
    assert_eq!(observed.retained_chunks, 111);
    assert_eq!(observed.producer_dropped_chunks, 2);
    assert_eq!(observed.producer_dropped_bytes, 3);
    assert_eq!(observed.provider_lagged_events, 4);
    assert_eq!(observed.storage_dropped_chunks, 5);
    assert_eq!(observed.storage_dropped_bytes, 6);
    assert_eq!(observed.evicted_chunks, 7);
    assert_eq!(observed.evicted_bytes, 8);
    assert_eq!(observed.earliest_retained_sequence, Some(10));
    assert!(!format!("{observed:?}").contains("reader-metadata-payload"));

    let first_page = reader
        .get_page(
            &owner,
            GatewayServiceLogReadRequest::new(scope, 100, None).expect("valid first page request"),
        )
        .await
        .expect("bounded byte page");
    assert_eq!(first_page.records.len(), 8);
    assert_eq!(first_page.records[0].sequence, 10);
    assert_eq!(first_page.records[7].sequence, 17);
    assert!(first_page.history_incomplete);
    assert_eq!(
        first_page
            .records
            .iter()
            .map(|record| record.bytes.len())
            .sum::<usize>(),
        458_775
    );
    let first_cursor = first_page.next_after.expect("byte-budget continuation");
    assert_eq!(first_cursor.sequence(), 17);

    let exact_byte_page = reader
        .get_page(
            &owner,
            GatewayServiceLogReadRequest::new(
                scope,
                100,
                Some(GatewayServiceLogReadCursor::new(scope, 10).expect("exact byte cursor")),
            )
            .expect("valid exact byte request"),
        )
        .await
        .expect("exact byte-budget page");
    assert_eq!(exact_byte_page.records.len(), 8);
    assert_eq!(exact_byte_page.records[0].sequence, 11);
    assert_eq!(exact_byte_page.records[7].sequence, 18);
    assert_eq!(
        exact_byte_page
            .records
            .iter()
            .map(|record| record.bytes.len())
            .sum::<usize>(),
        512 * 1024
    );
    assert_eq!(
        exact_byte_page
            .next_after
            .expect("ninth byte continuation")
            .sequence(),
        18
    );

    let gap_page = reader
        .get_page(
            &owner,
            GatewayServiceLogReadRequest::new(
                scope,
                100,
                Some(GatewayServiceLogReadCursor::new(scope, 9).expect("gap boundary cursor")),
            )
            .expect("valid gap boundary request"),
        )
        .await
        .expect("gap boundary page");
    assert!(!gap_page.history_incomplete);
    assert_eq!(gap_page.records[0].sequence, 10);

    let second_page = reader
        .get_page(
            &owner,
            GatewayServiceLogReadRequest::new(scope, 100, Some(first_cursor))
                .expect("valid continuation request"),
        )
        .await
        .expect("continuation page");
    assert_eq!(second_page.records.len(), 100);
    assert_eq!(second_page.records[0].sequence, 18);
    assert_eq!(second_page.records[99].sequence, 117);
    assert!(!second_page.history_incomplete);
    assert_eq!(
        second_page
            .next_after
            .expect("second continuation")
            .sequence(),
        117
    );

    let record_cursor = GatewayServiceLogReadCursor::new(scope, 19).expect("record cursor");
    let record_page = reader
        .get_page(
            &owner,
            GatewayServiceLogReadRequest::new(scope, 100, Some(record_cursor))
                .expect("valid record-cap request"),
        )
        .await
        .expect("record-cap page");
    assert_eq!(record_page.records.len(), 100);
    assert_eq!(record_page.records[0].sequence, 20);
    assert_eq!(record_page.records[99].sequence, 119);
    assert_eq!(
        record_page
            .next_after
            .expect("record continuation")
            .sequence(),
        119
    );

    let final_cursor = GatewayServiceLogReadCursor::new(scope, 119).expect("final cursor");
    let final_page = reader
        .get_page(
            &owner,
            GatewayServiceLogReadRequest::new(scope, 100, Some(final_cursor))
                .expect("valid final request"),
        )
        .await
        .expect("final page");
    assert_eq!(final_page.records.len(), 1);
    assert_eq!(final_page.records[0].sequence, 120);
    assert!(final_page.next_after.is_none());

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
        .fetch_one(&admin)
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
            &owner,
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
        .fetch_one(&admin)
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
            &owner,
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
            &owner,
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

    let foreign_scope = GatewayServiceLogReadScope {
        project_id: fixture.other_project,
        ..scope
    };
    let foreign_cursor =
        GatewayServiceLogReadCursor::new(foreign_scope, 0).expect("foreign cursor scope");
    let invalid_cursor_request = GatewayServiceLogReadRequest {
        scope,
        limit: 100,
        after: Some(foreign_cursor),
    };
    assert!(matches!(
        reader.get_page(&owner, invalid_cursor_request).await,
        Err(GatewayServiceLogReaderError::InvalidArgument)
    ));

    let wrong_project_page_identity = identity(fixture.owner, "wrong-project-page");
    assert_eq!(
        reader
            .get_page(
                &wrong_project_page_identity,
                GatewayServiceLogReadRequest::new(
                    GatewayServiceLogReadScope {
                        project_id: fixture.other_project,
                        ..scope
                    },
                    100,
                    None,
                )
                .expect("valid wrong-project page request"),
            )
            .await
            .map(|_| ()),
        Err(GatewayServiceLogReaderError::NotFound)
    );
    let wrong_project_page_audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
         WHERE request_id = $1 AND decision = 'allow'",
    )
    .bind(wrong_project_page_identity.request_id.as_uuid())
    .fetch_one(&admin)
    .await
    .expect("read wrong-project page audits");
    assert_eq!(wrong_project_page_audits, 2);

    let future_page_identity = identity(fixture.owner, "future-page");
    assert_eq!(
        reader
            .get_page(
                &future_page_identity,
                GatewayServiceLogReadRequest::new(
                    GatewayServiceLogReadScope {
                        fencing_token: 3,
                        ..scope
                    },
                    100,
                    None,
                )
                .expect("valid future-fence page request"),
            )
            .await
            .map(|_| ()),
        Err(GatewayServiceLogReaderError::NotFound)
    );
    let future_page_audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
         WHERE request_id = $1 AND decision = 'allow'",
    )
    .bind(future_page_identity.request_id.as_uuid())
    .fetch_one(&admin)
    .await
    .expect("read future-fence page audits");
    assert_eq!(future_page_audits, 2);

    let outsider_page_identity = identity(fixture.outsider, "outsider-page");
    assert_eq!(
        reader
            .get_page(
                &outsider_page_identity,
                GatewayServiceLogReadRequest::new(scope, 100, None)
                    .expect("valid outsider page request"),
            )
            .await
            .map(|_| ()),
        Err(GatewayServiceLogReaderError::Denied)
    );
    let outsider_page_denials: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
         WHERE request_id = $1 AND decision = 'deny' AND object_type = 'project'",
    )
    .bind(outsider_page_identity.request_id.as_uuid())
    .fetch_one(&admin)
    .await
    .expect("read outsider page audits");
    assert_eq!(outsider_page_denials, 1);

    sqlx::query("DELETE FROM project_maintainers WHERE project_id = $1 AND user_id = $2")
        .bind(fixture.project)
        .bind(fixture.member)
        .execute(&admin)
        .await
        .expect("revoke member page access");
    let member_page_identity = identity(fixture.member, "member-page");
    assert_eq!(
        reader
            .get_page(
                &member_page_identity,
                GatewayServiceLogReadRequest::new(scope, 100, None)
                    .expect("valid revoked-member page request"),
            )
            .await
            .map(|_| ()),
        Err(GatewayServiceLogReaderError::Denied)
    );
    let member_page_denials: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
         WHERE request_id = $1 AND decision = 'deny' AND object_type = 'project'",
    )
    .bind(member_page_identity.request_id.as_uuid())
    .fetch_one(&admin)
    .await
    .expect("read revoked-member page audits");
    assert_eq!(member_page_denials, 1);

    sqlx::query(
        "DELETE FROM gateway_service_log_chunks
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
    .execute(&admin)
    .await
    .expect("remove fully evicted log payloads");
    sqlx::query(
        "UPDATE gateway_service_log_epochs
            SET retained_bytes = 0, retained_chunks = 0
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
    .execute(&admin)
    .await
    .expect("mark fully evicted log metadata");
    let fully_evicted = reader
        .get_page(
            &owner,
            GatewayServiceLogReadRequest::new(scope, 100, None)
                .expect("valid fully evicted request"),
        )
        .await
        .expect("fully evicted page");
    assert!(fully_evicted.records.is_empty());
    assert!(fully_evicted.history_incomplete);
    assert_eq!(
        reader
            .get_epoch_metadata(&member, scope)
            .await
            .expect_err("revoked member metadata must be denied"),
        GatewayServiceLogReaderError::Denied
    );

    let wrong_project_identity = identity(fixture.owner, "wrong-project");
    assert_eq!(
        reader
            .get_epoch_metadata(
                &wrong_project_identity,
                GatewayServiceLogReadScope {
                    project_id: fixture.other_project,
                    ..scope
                },
            )
            .await,
        Err(GatewayServiceLogReaderError::NotFound)
    );
    let wrong_project_audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
         WHERE request_id = $1 AND decision = 'allow'",
    )
    .bind(wrong_project_identity.request_id.as_uuid())
    .fetch_one(&admin)
    .await
    .expect("read authorized not-found audit");
    assert_eq!(wrong_project_audits, 2);

    for mismatched in [
        GatewayServiceLogReadScope {
            project_id: Uuid::new_v4(),
            ..scope
        },
        GatewayServiceLogReadScope {
            gateway_id: Uuid::new_v4(),
            ..scope
        },
        GatewayServiceLogReadScope {
            revision_id: Uuid::new_v4(),
            ..scope
        },
        GatewayServiceLogReadScope {
            instance_id: Uuid::new_v4(),
            ..scope
        },
        GatewayServiceLogReadScope {
            fencing_token: 3,
            ..scope
        },
    ] {
        assert!(matches!(
            reader.get_epoch_metadata(&owner, mismatched).await,
            Err(GatewayServiceLogReaderError::Denied | GatewayServiceLogReaderError::NotFound)
        ));
    }

    assert_eq!(
        reader.get_epoch_metadata(&outsider, scope).await,
        Err(GatewayServiceLogReaderError::Denied)
    );
    assert_eq!(
        reader.get_epoch_metadata(&member, scope).await,
        Err(GatewayServiceLogReaderError::Denied)
    );

    let (owner_allows, outsider_denies, member_denies): (i64, i64, i64) = sqlx::query_as(
        "SELECT
            count(*) FILTER (WHERE actor_id = $1 AND decision = 'allow'),
            count(*) FILTER (WHERE actor_id = $2 AND decision = 'deny'),
            count(*) FILTER (WHERE actor_id = $3 AND decision = 'deny')
         FROM authorization_audit_events
         WHERE actor_id IN ($1, $2, $3)",
    )
    .bind(fixture.owner)
    .bind(fixture.outsider)
    .bind(fixture.member)
    .fetch_one(&admin)
    .await
    .expect("read persisted reader audit decisions");
    assert!(owner_allows >= 4);
    assert!(outsider_denies >= 1);
    assert!(member_denies >= 1);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn project_metadata_is_scoped_audited_and_preserved_by_worker_gc() {
    let Some(admin) = test_pool().await else {
        return;
    };
    let max_version: i64 = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(version) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&admin)
    .await
    .expect("read migration marker")
    .expect("migrations exist");
    assert!(max_version >= 81);
    println!("REAL_POSTGRES_PROJECT_METADATA=1 max_migration={max_version}");
    let fixture = seed_fixture(&admin).await;
    let app = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SET application_name = 'heph-service-log-project-metadata'")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("test database URL"))
        .await
        .expect("connect application role");
    let current_user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&app)
        .await
        .expect("read application role");
    assert_eq!(current_user, "hephaestus_app");
    let reader = PostgresGatewayServiceLogReader::new(
        app.clone(),
        Arc::new(authz_postgres::PostgresMelangeAuthorizer),
    );
    let owner = identity(fixture.owner, "project-metadata-owner");
    let member = identity(fixture.member, "project-metadata-member");

    // This project has no usage row; the authorized aggregate is an explicit
    // empty value rather than a fabricated usage record.
    let absent = reader
        .get_project_metadata(&owner, fixture.other_project)
        .await
        .expect("authorized project without usage row");
    assert!(!absent.usage_present);
    assert_eq!(absent.storage_dropped_chunks, 0);
    assert_eq!(absent.storage_dropped_bytes, 0);

    let project_b_owner = Uuid::new_v4();
    let project_b_organization = Uuid::new_v4();
    let project_b = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
        .bind(project_b_owner)
        .bind("reader project B owner")
        .execute(&admin)
        .await
        .expect("insert project B owner");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(project_b_organization)
        .bind(format!("reader-project-b-org-{project_b_organization}"))
        .execute(&admin)
        .await
        .expect("insert project B organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(project_b_organization)
    .bind(project_b_owner)
    .execute(&admin)
    .await
    .expect("insert project B organization owner");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project_b)
        .bind(project_b_organization)
        .bind(format!("reader-project-b-{project_b}"))
        .execute(&admin)
        .await
        .expect("insert project B");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project_b)
        .bind(project_b_owner)
        .execute(&admin)
        .await
        .expect("insert project B maintainer");
    sqlx::query(
        "INSERT INTO gateway_service_log_project_usage
            (project_id, retained_epochs, storage_dropped_chunks,
             storage_dropped_bytes)
         VALUES ($1, 1, 29, 2901), ($2, 0, 31, 3101)",
    )
    .bind(fixture.project)
    .bind(project_b)
    .execute(&admin)
    .await
    .expect("seed distinct project loss counters");

    let project_metadata = reader
        .get_project_metadata(&owner, fixture.project)
        .await
        .expect("owner project metadata");
    assert!(project_metadata.usage_present);
    assert_eq!(project_metadata.storage_dropped_chunks, 29);
    assert_eq!(project_metadata.storage_dropped_bytes, 2901);
    let owner_audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
          WHERE request_id = $1 AND object_type = 'project'
            AND object_id = $2 AND permission = 'can_read' AND decision = 'allow'",
    )
    .bind(owner.request_id.as_uuid())
    .bind(fixture.project)
    .fetch_one(&admin)
    .await
    .expect("read owner project audit");
    assert_eq!(owner_audit_count, 1);

    let member_metadata = reader
        .get_project_metadata(&member, fixture.project)
        .await
        .expect("member project metadata before revocation");
    assert_eq!(member_metadata.storage_dropped_chunks, 29);
    assert_eq!(member_metadata.storage_dropped_bytes, 2901);
    let member_allow_request = member.request_id;
    let member_allow_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
          WHERE request_id = $1 AND object_type = 'project'
            AND object_id = $2 AND decision = 'allow'",
    )
    .bind(member_allow_request.as_uuid())
    .bind(fixture.project)
    .fetch_one(&admin)
    .await
    .expect("read member allow audit");
    assert_eq!(member_allow_count, 1);

    let denied_owner = identity(fixture.owner, "project-b-denied");
    assert_eq!(
        reader.get_project_metadata(&denied_owner, project_b).await,
        Err(GatewayServiceLogReaderError::Denied)
    );
    let denied_owner_audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
          WHERE request_id = $1 AND object_type = 'project'
            AND object_id = $2 AND permission = 'can_read' AND decision = 'deny'",
    )
    .bind(denied_owner.request_id.as_uuid())
    .bind(project_b)
    .fetch_one(&admin)
    .await
    .expect("read denied project audit");
    assert_eq!(denied_owner_audit_count, 1);

    let project_b_identity = identity(project_b_owner, "project-b-owner");
    let project_b_metadata = reader
        .get_project_metadata(&project_b_identity, project_b)
        .await
        .expect("project B owner metadata");
    assert_eq!(project_b_metadata.storage_dropped_chunks, 31);
    assert_eq!(project_b_metadata.storage_dropped_bytes, 3101);

    sqlx::query("DELETE FROM project_maintainers WHERE project_id = $1 AND user_id = $2")
        .bind(fixture.project)
        .bind(fixture.member)
        .execute(&admin)
        .await
        .expect("revoke member project metadata access");
    let revoked_member = identity(fixture.member, "project-metadata-member-revoked");
    assert_eq!(
        reader
            .get_project_metadata(&revoked_member, fixture.project)
            .await,
        Err(GatewayServiceLogReaderError::Denied)
    );
    let revoked_member_audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
          WHERE request_id = $1 AND object_type = 'project'
            AND object_id = $2 AND permission = 'can_read' AND decision = 'deny'",
    )
    .bind(revoked_member.request_id.as_uuid())
    .bind(fixture.project)
    .fetch_one(&admin)
    .await
    .expect("read revoked member audit");
    assert_eq!(revoked_member_audit_count, 1);

    let worker = worker_pool().await;
    let gc_owner = gateway_edge::GatewayServiceOwner::new(&fixture.owner_host_id, Uuid::new_v4())
        .expect("valid GC owner");
    let ownership = gateway_postgres::PostgresGatewayServiceOwnership::new(worker.clone());
    let recovered = ownership
        .claim_expired(&gc_owner, Duration::from_secs(30), 1)
        .await
        .expect("claim expired fixture instance");
    assert_eq!(recovered.len(), 1);
    ownership
        .mark_cleaned(&recovered[0], &gc_owner)
        .await
        .expect("mark fixture instance cleaned");
    sqlx::query(
        "INSERT INTO gateway_service_log_epochs
            (instance_id, gateway_id, revision_id, project_id, fencing_token,
             acknowledged_through, retained_bytes, retained_chunks, updated_at)
         VALUES ($1, $2, $3, $4, 1, 12, 0, 0, now() - interval '2 days')",
    )
    .bind(fixture.instance)
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(fixture.project)
    .execute(&admin)
    .await
    .expect("seed aged cleaned epoch");

    let worker_store = gateway_postgres::PostgresGatewayServiceLogStore::new(worker.clone());
    let report = gateway_edge::GatewayServiceLogMaintenance::maintain_project(
        &worker_store,
        fixture.project,
        gateway_edge::GatewayServiceLogMaintenancePolicy::default(),
    )
    .await
    .expect("worker maintenance removes aged cleaned epoch");
    assert_eq!(report.metadata_epochs, 1);
    let remaining_epochs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_service_log_epochs WHERE project_id = $1")
            .bind(fixture.project)
            .fetch_one(&worker)
            .await
            .expect("read worker epoch count");
    assert_eq!(remaining_epochs, 0);
    let preserved_loss: (i64, i64) = sqlx::query_as(
        "SELECT storage_dropped_chunks, storage_dropped_bytes
           FROM gateway_service_log_project_usage WHERE project_id = $1",
    )
    .bind(fixture.project)
    .fetch_one(&worker)
    .await
    .expect("worker reads preserved project loss");
    assert_eq!(preserved_loss, (29, 2901));
    let preserved_metadata = reader
        .get_project_metadata(&owner, fixture.project)
        .await
        .expect("reader preserves project loss after worker GC");
    assert!(preserved_metadata.usage_present);
    assert_eq!(preserved_metadata.storage_dropped_chunks, 29);
    assert_eq!(preserved_metadata.storage_dropped_bytes, 2901);

    // These counters are seeded metadata; this test does not prove append-cap
    // rejection increments, which belongs to the append/reader proof.
    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, false)")
        .bind(fixture.owner.to_string())
        .execute(&app)
        .await
        .expect("set direct RLS actor");
    let visible_projects: Vec<Uuid> = sqlx::query_scalar(
        "SELECT project_id FROM gateway_service_log_project_usage
          WHERE project_id IN ($1, $2) ORDER BY project_id",
    )
    .bind(fixture.project)
    .bind(project_b)
    .fetch_all(&app)
    .await
    .expect("read actor-scoped project usage");
    assert_eq!(visible_projects, vec![fixture.project]);
    let retained_error = sqlx::query(
        "SELECT retained_bytes FROM gateway_service_log_project_usage
          WHERE project_id = $1",
    )
    .bind(fixture.project)
    .fetch_one(&app)
    .await
    .expect_err("application role must not read retained usage column");
    assert_eq!(
        retained_error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("42501")
    );
    let worker_loss: (i64, i64) = sqlx::query_as(
        "SELECT storage_dropped_chunks, storage_dropped_bytes
           FROM gateway_service_log_project_usage WHERE project_id = $1",
    )
    .bind(fixture.project)
    .fetch_one(&worker)
    .await
    .expect("worker retained usage access");
    assert_eq!(worker_loss, (29, 2901));
    app.close().await;
    worker.close().await;
}

#[derive(Clone)]
struct Fixture {
    owner: Uuid,
    member: Uuid,
    outsider: Uuid,
    project: Uuid,
    other_project: Uuid,
    gateway: Uuid,
    revision: Uuid,
    instance: Uuid,
    owner_host_id: String,
}

fn identity(user: Uuid, label: &str) -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        UserId::from_uuid(user),
        "reader-test",
        label,
        serde_json::json!({}),
        RequestId::new(),
    )
}

// Keep the disposable graph in one fixture so each authorization assertion
// uses the same exact project/gateway/revision/instance identity.
#[allow(clippy::too_many_lines)]
async fn seed_fixture(pool: &sqlx::PgPool) -> Fixture {
    let owner = Uuid::new_v4();
    let member = Uuid::new_v4();
    let outsider = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let other_project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let gateway = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let instance = Uuid::new_v4();
    let owner_host_id = format!("reader-host-{instance}");

    for (id, name) in [
        (owner, "reader owner"),
        (member, "reader member"),
        (outsider, "reader outsider"),
    ] {
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
            .bind(id)
            .bind(name)
            .execute(pool)
            .await
            .expect("insert reader user");
    }
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization)
        .bind(format!("reader-org-{organization}"))
        .execute(pool)
        .await
        .expect("insert reader organization");
    sqlx::query("INSERT INTO organization_members (organization_id, user_id, role) VALUES ($1, $2, 'owner')")
        .bind(organization)
        .bind(owner)
        .execute(pool)
        .await
        .expect("insert organization owner");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project)
        .bind(organization)
        .bind(format!("reader-project-{project}"))
        .execute(pool)
        .await
        .expect("insert reader project");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(other_project)
        .bind(organization)
        .bind(format!("reader-other-project-{other_project}"))
        .execute(pool)
        .await
        .expect("insert reader other project");
    for user in [owner, member] {
        sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
            .bind(project)
            .bind(user)
            .execute(pool)
            .await
            .expect("insert reader project maintainer");
    }
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(other_project)
        .bind(owner)
        .execute(pool)
        .await
        .expect("insert reader other project maintainer");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository)
        .bind(project)
        .bind(format!("reader-repository-{repository}"))
        .execute(pool)
        .await
        .expect("insert reader repository");
    sqlx::query(
        "INSERT INTO gateways (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, $4, 'paused', $5)",
    )
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(format!(
        "reader-gateway-{}",
        &gateway.simple().to_string()[..8]
    ))
    .bind(owner)
    .execute(pool)
    .await
    .expect("insert reader gateway");
    sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, handler_contract, exposure,
             parameters, secret_slots, normalized_hash, created_by,
             service_loopback_port, service_readiness_path, service_health_path,
             service_log_capture_mode)
         VALUES ($1, $2, $3, $4, 'http.service.v1', 'public', '{}', '{}', $5, $6,
                 8080, '/readyz', '/healthz', 'application')",
    )
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind([7_u8; 32].as_slice())
    .bind(owner)
    .execute(pool)
    .await
    .expect("insert reader revision");
    sqlx::query(
        "INSERT INTO gateway_service_instances
            (id, gateway_id, revision_id, owner_host_id, owner_uuid, fencing_token,
             vm_id, state, lease_expires_at, heartbeat_at)
         VALUES ($1, $2, $3, $6, $4, 1, $5, 'ready',
                 now() - interval '1 minute', now() - interval '2 minutes')",
    )
    .bind(instance)
    .bind(gateway)
    .bind(revision)
    .bind(Uuid::new_v4())
    .bind(format!("gateway-service-{instance}"))
    .bind(&owner_host_id)
    .execute(pool)
    .await
    .expect("insert reader service instance");

    Fixture {
        owner,
        member,
        outsider,
        project,
        other_project,
        gateway,
        revision,
        instance,
        owner_host_id,
    }
}

async fn test_pool() -> Option<sqlx::PgPool> {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply real PostgreSQL migrations");
    Some(pool)
}

async fn worker_pool() -> sqlx::PgPool {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("worker test database URL");
    PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SET application_name = 'heph-service-log-project-metadata-worker'")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect worker role")
}
