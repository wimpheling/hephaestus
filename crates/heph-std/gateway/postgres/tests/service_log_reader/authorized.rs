//! Authorized epoch metadata, pagination, and audit scenario.

use super::authorization::verify_authorization;
use super::pagination::verify_pagination;
use super::retention::verify_retention;
use super::snapshot::verify_snapshot;
use super::support::{identity, seed_fixture};
use gateway_domain::{
    GatewayServiceLogReadCursor, GatewayServiceLogReadMetadata, GatewayServiceLogReadRequest,
    GatewayServiceLogReadScope,
};
use gateway_postgres::PostgresGatewayServiceLogReader;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::{env, sync::Arc};
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
    sqlx::migrate!("../../../../migrations")
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
    let repository_error = sqlx::query("SELECT repository_id FROM gateways LIMIT 1")
        .fetch_optional(&app)
        .await
        .expect_err("application role must not read ungranted gateway columns");
    let database_error = repository_error
        .as_database_error()
        .expect("permission failure must be a database error");
    assert_eq!(database_error.code().as_deref(), Some("42501"));
    let project_error = sqlx::query("SELECT project_id FROM gateway_revisions LIMIT 1")
        .fetch_optional(&app)
        .await
        .expect_err("application role must not read ungranted gateway columns");
    let database_error = project_error
        .as_database_error()
        .expect("permission failure must be a database error");
    assert_eq!(database_error.code().as_deref(), Some("42501"));

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

    verify_pagination(&reader, scope, &owner, first_cursor).await;
    verify_snapshot(&admin, &reader, &fixture, scope, &owner).await;
    verify_retention(&admin, &reader, &fixture, scope, &owner).await;
    verify_authorization(&admin, &reader, &fixture, scope, &owner, &member, &outsider).await;
}
