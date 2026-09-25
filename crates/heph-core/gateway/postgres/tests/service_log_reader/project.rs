//! Project metadata and worker cleanup scenario.

use super::support::{identity, seed_fixture, test_pool, worker_pool};
use gateway_domain::GatewayServiceOwnership;
use gateway_postgres::{GatewayServiceLogReaderError, PostgresGatewayServiceLogReader};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::{env, sync::Arc, time::Duration};
use uuid::Uuid;

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
    let gc_owner = gateway_domain::GatewayServiceOwner::new(&fixture.owner_host_id, Uuid::new_v4())
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
    let report = gateway_domain::GatewayServiceLogMaintenance::maintain_project(
        &worker_store,
        fixture.project,
        gateway_domain::GatewayServiceLogMaintenancePolicy::default(),
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
