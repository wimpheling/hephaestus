//! Real `PostgreSQL` coverage for authorized service-log epoch metadata reads.

use gateway_edge::{GatewayServiceLogReadMetadata, GatewayServiceLogReadScope};
use gateway_postgres::{GatewayServiceLogReaderError, PostgresGatewayServiceLogReader};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
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
    assert!(max_version >= 80);
    println!("REAL_POSTGRES_LOG_READER=1 max_migration={max_version}");

    let fixture = seed_fixture(&admin).await;
    let app = PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(connection)
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

    sqlx::query(
        "INSERT INTO gateway_service_log_epochs
            (instance_id, gateway_id, revision_id, project_id, fencing_token,
             acknowledged_through, retained_bytes, retained_chunks,
             producer_dropped_chunks, producer_dropped_bytes,
             provider_lagged_events, storage_dropped_chunks, storage_dropped_bytes,
             evicted_chunks, evicted_bytes)
         VALUES ($1, $2, $3, $4, 1, 10, 23, 1, 2, 3, 4, 5, 6, 7, 8)",
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
    assert_eq!(observed.acknowledged_through, Some(10));
    assert_eq!(observed.retained_bytes, 23);
    assert_eq!(observed.retained_chunks, 1);
    assert_eq!(observed.producer_dropped_chunks, 2);
    assert_eq!(observed.producer_dropped_bytes, 3);
    assert_eq!(observed.provider_lagged_events, 4);
    assert_eq!(observed.storage_dropped_chunks, 5);
    assert_eq!(observed.storage_dropped_bytes, 6);
    assert_eq!(observed.evicted_chunks, 7);
    assert_eq!(observed.evicted_bytes, 8);
    assert_eq!(observed.earliest_retained_sequence, Some(10));
    assert!(!format!("{observed:?}").contains("reader-metadata-payload"));
    assert_eq!(
        reader
            .get_epoch_metadata(&member, scope)
            .await
            .expect("authorized member metadata"),
        observed
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
    sqlx::query("DELETE FROM project_maintainers WHERE project_id = $1 AND user_id = $2")
        .bind(fixture.project)
        .bind(fixture.member)
        .execute(&admin)
        .await
        .expect("revoke member project access");
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

#[derive(Clone, Copy)]
struct Fixture {
    owner: Uuid,
    member: Uuid,
    outsider: Uuid,
    project: Uuid,
    other_project: Uuid,
    gateway: Uuid,
    revision: Uuid,
    instance: Uuid,
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
         VALUES ($1, $2, $3, 'reader-host', $4, 1, $5, 'ready',
                 now() - interval '1 minute', now() - interval '2 minutes')",
    )
    .bind(instance)
    .bind(gateway)
    .bind(revision)
    .bind(Uuid::new_v4())
    .bind(format!("gateway-service-{instance}"))
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
    }
}
