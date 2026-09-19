//! Real `PostgreSQL` coverage for durable gateway service ownership.

use gateway_edge::{
    GatewayServiceInstanceState, GatewayServiceOwner, GatewayServiceOwnership,
    GatewayServiceOwnershipError, MAX_SERVICE_OWNERSHIP_BATCH,
};
use gateway_postgres::PostgresGatewayServiceOwnership;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::{env, sync::Arc, time::Duration};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_claims_one_live_service_and_fences_stale_owner() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = Arc::new(PostgresGatewayServiceOwnership::new(pool.clone()));
    let first = GatewayServiceOwner::new("ownership-host", Uuid::new_v4()).expect("owner");
    let second = GatewayServiceOwner::new("ownership-host", Uuid::new_v4()).expect("owner");
    let (left, right) = tokio::join!(
        ownership.claim_new(
            fixture.gateway,
            fixture.revision,
            &first,
            Duration::from_millis(1),
        ),
        ownership.claim_new(
            fixture.gateway,
            fixture.revision,
            &second,
            Duration::from_millis(1),
        )
    );
    let (first_claim, old_owner) = match (left, right) {
        (Ok(claim), Err(GatewayServiceOwnershipError::Conflict)) => (claim, first),
        (Err(GatewayServiceOwnershipError::Conflict), Ok(claim)) => (claim, second),
        _ => panic!("concurrent claim did not produce one winner"),
    };
    assert_eq!(first_claim.state, GatewayServiceInstanceState::Provisioning);
    assert_eq!(
        first_claim.vm_id,
        format!("gateway-service-{}", first_claim.identity.instance_id)
    );

    tokio::time::sleep(Duration::from_millis(10)).await;
    let recovery_owner = old_owner.clone();
    let recovered = ownership
        .claim_expired(&recovery_owner, Duration::from_secs(30), 1)
        .await
        .expect("recover claim");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].fencing_token, first_claim.fencing_token + 1);
    assert_eq!(recovered[0].state, GatewayServiceInstanceState::Stopping);
    assert_eq!(recovered[0].owner_uuid, old_owner.owner_uuid);
    assert!(matches!(
        ownership
            .renew(&first_claim, &old_owner, Duration::from_secs(30))
            .await,
        Err(GatewayServiceOwnershipError::StaleLease)
    ));
    ownership
        .mark_cleaned(&recovered[0], &recovery_owner)
        .await
        .expect("clean recovered claim");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_requires_exact_service_revision_and_host() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let service = seed_fixture(&pool, "http.service.v1").await;
    sqlx::query(
        "UPDATE gateways
            SET active_revision_id = NULL, desired_service_revision_id = $2
          WHERE id = $1",
    )
    .bind(service.gateway)
    .bind(service.revision)
    .execute(&pool)
    .await
    .expect("pending service revision");
    let stateless = seed_fixture(&pool, "http.v1").await;
    let ownership = PostgresGatewayServiceOwnership::new(pool.clone());
    let owner = GatewayServiceOwner::new("ownership-host", Uuid::new_v4()).expect("owner");
    let mut invalid_owner = owner.clone();
    invalid_owner.host_id = "ownership host".to_owned();
    assert!(matches!(
        ownership
            .claim_new(
                service.gateway,
                service.revision,
                &invalid_owner,
                Duration::from_secs(30),
            )
            .await,
        Err(GatewayServiceOwnershipError::InvalidArgument)
    ));
    assert!(matches!(
        ownership
            .claim_new(
                Uuid::nil(),
                service.revision,
                &owner,
                Duration::from_secs(30),
            )
            .await,
        Err(GatewayServiceOwnershipError::InvalidArgument)
    ));
    assert!(matches!(
        ownership
            .claim_new(
                stateless.gateway,
                stateless.revision,
                &owner,
                Duration::from_secs(30),
            )
            .await,
        Err(GatewayServiceOwnershipError::Conflict)
    ));
    ownership
        .claim_new(
            service.gateway,
            service.revision,
            &owner,
            Duration::from_millis(1),
        )
        .await
        .expect("service claim");

    let active_and_pending = seed_fixture(&pool, "http.service.v1").await;
    let pending_revision = seed_service_revision(&pool, active_and_pending.gateway).await;
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(active_and_pending.gateway)
        .bind(pending_revision)
        .execute(&pool)
        .await
        .expect("pending replacement revision");
    ownership
        .claim_new(
            active_and_pending.gateway,
            active_and_pending.revision,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("active service remains claimable during pending replacement");
    ownership
        .claim_new(
            active_and_pending.gateway,
            pending_revision,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("pending service replacement claim");
    tokio::time::sleep(Duration::from_millis(10)).await;
    let other_host = GatewayServiceOwner::new("other-host", Uuid::new_v4()).expect("owner");
    assert!(
        ownership
            .claim_expired(&other_host, Duration::from_secs(30), 1)
            .await
            .expect("other-host recovery")
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_renewal_checks_expiry_after_waiting_for_instance_lock() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = PostgresGatewayServiceOwnership::new(pool.clone());
    let owner = GatewayServiceOwner::new("lock-host", Uuid::new_v4()).expect("owner");
    let claim = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &owner,
            Duration::from_millis(30),
        )
        .await
        .expect("claim");
    let mut blocker = pool.begin().await.expect("blocker transaction");
    sqlx::query("SELECT id FROM gateway_service_instances WHERE id = $1 FOR UPDATE")
        .bind(claim.identity.instance_id)
        .execute(&mut *blocker)
        .await
        .expect("lock instance");
    let renewal = tokio::spawn({
        let ownership = ownership.clone();
        let claim = claim.clone();
        let owner = owner.clone();
        async move {
            ownership
                .renew(&claim, &owner, Duration::from_secs(30))
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(60)).await;
    blocker.commit().await.expect("release instance lock");
    assert!(matches!(
        renewal.await.expect("renewal task"),
        Err(GatewayServiceOwnershipError::StaleLease)
    ));
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_cleanup_failure_keeps_live_uniqueness_and_batch_is_bounded() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let ownership = PostgresGatewayServiceOwnership::new(pool.clone());
    let owner = GatewayServiceOwner::new("batch-host", Uuid::new_v4()).expect("owner");
    let mut fixtures = Vec::with_capacity(MAX_SERVICE_OWNERSHIP_BATCH + 2);
    for _ in 0..MAX_SERVICE_OWNERSHIP_BATCH + 2 {
        fixtures.push(seed_fixture(&pool, "http.service.v1").await);
    }
    let mut claims = Vec::with_capacity(fixtures.len());
    for fixture in &fixtures {
        claims.push(
            ownership
                .claim_new(
                    fixture.gateway,
                    fixture.revision,
                    &owner,
                    Duration::from_millis(1),
                )
                .await
                .expect("batch claim"),
        );
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
    let recovery_owner = GatewayServiceOwner::new("batch-host", Uuid::new_v4()).expect("owner");
    let first = ownership
        .claim_expired(
            &recovery_owner,
            Duration::from_secs(30),
            MAX_SERVICE_OWNERSHIP_BATCH,
        )
        .await
        .expect("first batch");
    assert_eq!(first.len(), MAX_SERVICE_OWNERSHIP_BATCH);
    let second = ownership
        .claim_expired(
            &recovery_owner,
            Duration::from_secs(30),
            MAX_SERVICE_OWNERSHIP_BATCH,
        )
        .await
        .expect("second batch");
    assert_eq!(second.len(), 2);

    let first_recovered = first.first().expect("recovered claim");
    let cleanup_claim = ownership
        .claim_new(
            first_recovered.identity.gateway_id,
            first_recovered.identity.revision_id,
            &owner,
            Duration::from_secs(30),
        )
        .await;
    assert!(matches!(
        cleanup_claim,
        Err(GatewayServiceOwnershipError::Conflict)
    ));
    ownership
        .mark_cleaned(first_recovered, &recovery_owner)
        .await
        .expect("mark cleaned");
    ownership
        .claim_new(
            first_recovered.identity.gateway_id,
            first_recovered.identity.revision_id,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("claim after cleanup");
    drop(claims);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_sql_trigger_rejects_identity_and_fence_bypass() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = PostgresGatewayServiceOwnership::new(pool.clone());
    let owner = GatewayServiceOwner::new("trigger-host", Uuid::new_v4()).expect("owner");
    let claim = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("claim");
    let identity_error = sqlx::query(
        "UPDATE gateway_service_instances
            SET gateway_id = $2
          WHERE id = $1",
    )
    .bind(claim.identity.instance_id)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await;
    assert!(identity_error.is_err());
    let fence_error = sqlx::query(
        "UPDATE gateway_service_instances
            SET fencing_token = fencing_token + 1
          WHERE id = $1",
    )
    .bind(claim.identity.instance_id)
    .execute(&pool)
    .await;
    assert!(fence_error.is_err());
    let transition_error = sqlx::query(
        "UPDATE gateway_service_instances
            SET state = 'ready'
          WHERE id = $1",
    )
    .bind(claim.identity.instance_id)
    .execute(&pool)
    .await;
    assert!(transition_error.is_err());
}

#[derive(Clone, Copy)]
struct Fixture {
    gateway: Uuid,
    revision: Uuid,
}

async fn test_pool() -> Option<sqlx::PgPool> {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(16)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply gateway migrations");
    let max_version: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("read latest migration");
    let max_version = max_version.expect("migrations are present");
    assert!(max_version >= 74);
    println!("REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration={max_version}");
    Some(pool)
}

async fn seed_fixture(pool: &sqlx::PgPool, contract: &str) -> Fixture {
    let owner = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let gateway = Uuid::new_v4();
    let revision = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'ownership owner')")
        .bind(owner)
        .execute(pool)
        .await
        .expect("owner");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization)
        .bind(format!("ownership-{organization}"))
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project)
        .bind(organization)
        .bind(format!("ownership-{project}"))
        .execute(pool)
        .await
        .expect("project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository)
        .bind(project)
        .bind(format!("ownership-{repository}"))
        .execute(pool)
        .await
        .expect("repository");
    sqlx::query(
        "INSERT INTO gateways
            (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, $4, 'enabled', $5)",
    )
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(format!("ownership-{}", &gateway.to_string()[..8]))
    .bind(owner)
    .execute(pool)
    .await
    .expect("gateway");
    let service = contract == "http.service.v1";
    let (port, readiness, health, slots) = if service {
        ("18080", "'/ready'", "'/health'", "'{hook}'")
    } else {
        ("NULL", "NULL", "NULL", "'{}'")
    };
    let query = format!(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, handler_contract, exposure,
             parameters, secret_slots, service_loopback_port, service_readiness_path,
             service_health_path, normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5, 'public', '{{}}', {slots}, {port}, {readiness},
                 {health}, $6, $7)"
    );
    sqlx::query(&query)
        .bind(revision)
        .bind(gateway)
        .bind(project)
        .bind(repository)
        .bind(contract)
        .bind([9_u8; 32].as_slice())
        .bind(owner)
        .execute(pool)
        .await
        .expect("revision");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway)
        .bind(revision)
        .execute(pool)
        .await
        .expect("active revision");
    Fixture { gateway, revision }
}

async fn seed_service_revision(pool: &sqlx::PgPool, gateway: Uuid) -> Uuid {
    let revision = Uuid::new_v4();
    let query = "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, handler_contract, exposure,
             parameters, secret_slots, service_loopback_port, service_readiness_path,
             service_health_path, normalized_hash, created_by)
         SELECT $2, gateway.id, gateway.project_id, gateway.repository_id,
                'http.service.v1', 'public', '{}', '{hook}', 18081, '/ready',
                '/health', $3, gateway.created_by
           FROM gateways AS gateway
          WHERE gateway.id = $1";
    let result = sqlx::query(query)
        .bind(gateway)
        .bind(revision)
        .bind([8_u8; 32].as_slice())
        .execute(pool)
        .await
        .expect("replacement service revision");
    assert_eq!(result.rows_affected(), 1);
    revision
}
