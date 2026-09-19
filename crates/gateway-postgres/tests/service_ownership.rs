//! Real `PostgreSQL` coverage for durable gateway service ownership.

use gateway_edge::{
    GatewayServiceFailure, GatewayServiceFailureCode, GatewayServiceFailureStore,
    GatewayServiceFailureStoreError, GatewayServiceInstanceState, GatewayServiceOwner,
    GatewayServiceOwnership, GatewayServiceOwnershipError, MAX_SERVICE_OWNERSHIP_BATCH,
};
use gateway_postgres::{PostgresGatewayServiceFailureStore, PostgresGatewayServiceOwnership};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::{env, sync::Arc, time::Duration};
use uuid::Uuid;

#[path = "service_ownership/claim_resolution.rs"]
mod claim_resolution;
#[path = "service_ownership/coordinator.rs"]
mod coordinator;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_claims_one_live_service_and_fences_stale_owner() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = Arc::new(worker_ownership().await);
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
    let ownership = worker_ownership().await;
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
    let ownership = worker_ownership().await;
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
    wait_for_lock(&pool, "gateway_service_instances").await;
    tokio::time::sleep(Duration::from_millis(60)).await;
    blocker.commit().await.expect("release instance lock");
    assert!(matches!(
        renewal.await.expect("renewal task"),
        Err(GatewayServiceOwnershipError::StaleLease)
    ));
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_transitions_and_promotes_pending_service() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let candidate = seed_service_revision(&pool, fixture.gateway).await;
    sqlx::query(
        "UPDATE gateways
            SET active_revision_id = NULL, desired_service_revision_id = $2
          WHERE id = $1",
    )
    .bind(fixture.gateway)
    .bind(candidate)
    .execute(&pool)
    .await
    .expect("pending candidate pointers");
    let before_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
          WHERE aggregate_type = 'gateway' AND aggregate_id = $1",
    )
    .bind(fixture.gateway)
    .fetch_one(&pool)
    .await
    .expect("promotion event baseline");
    let ownership = worker_ownership().await;
    let owner = GatewayServiceOwner::new("promotion-host", Uuid::new_v4()).expect("owner");
    let claim = ownership
        .claim_new(fixture.gateway, candidate, &owner, Duration::from_secs(30))
        .await
        .expect("candidate claim");
    assert!(matches!(
        ownership.mark_ready(&claim, &owner).await,
        Err(GatewayServiceOwnershipError::Conflict)
    ));
    let starting = ownership
        .mark_starting(&claim, &owner)
        .await
        .expect("starting transition");
    assert!(matches!(
        ownership.mark_starting(&starting, &owner).await,
        Err(GatewayServiceOwnershipError::Conflict)
    ));
    let ready = ownership
        .mark_ready(&starting, &owner)
        .await
        .expect("ready transition");
    assert_eq!(ready.state, GatewayServiceInstanceState::Ready);
    let previous = ownership
        .promote_ready(&ready, &owner)
        .await
        .expect("promote initial candidate");
    assert_eq!(previous, None);
    let pointers: (Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        "SELECT active_revision_id, desired_service_revision_id
           FROM gateways WHERE id = $1",
    )
    .bind(fixture.gateway)
    .fetch_one(&pool)
    .await
    .expect("promoted pointers");
    assert_eq!(pointers, (Some(candidate), Some(candidate)));
    let after_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
          WHERE aggregate_type = 'gateway' AND aggregate_id = $1",
    )
    .bind(fixture.gateway)
    .fetch_one(&pool)
    .await
    .expect("promotion event count");
    assert_eq!(after_events, before_events + 1);
    assert_eq!(
        ownership
            .promote_ready(&ready, &owner)
            .await
            .expect("idempotent promotion retry"),
        Some(candidate)
    );
    let retry_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
          WHERE aggregate_type = 'gateway' AND aggregate_id = $1",
    )
    .bind(fixture.gateway)
    .fetch_one(&pool)
    .await
    .expect("idempotent event count");
    assert_eq!(retry_events, after_events);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_old_active_drains_only_after_candidate_promotion() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let candidate = seed_service_revision(&pool, fixture.gateway).await;
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway)
        .bind(candidate)
        .execute(&pool)
        .await
        .expect("desired candidate");
    let ownership = worker_ownership().await;
    let owner = GatewayServiceOwner::new("cutover-host", Uuid::new_v4()).expect("owner");
    let old = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("old claim");
    let old = ownership
        .mark_ready(
            &ownership
                .mark_starting(&old, &owner)
                .await
                .expect("old starting"),
            &owner,
        )
        .await
        .expect("old ready");
    let candidate_claim = ownership
        .claim_new(fixture.gateway, candidate, &owner, Duration::from_secs(30))
        .await
        .expect("candidate claim");
    let candidate_ready = ownership
        .mark_ready(
            &ownership
                .mark_starting(&candidate_claim, &owner)
                .await
                .expect("candidate starting"),
            &owner,
        )
        .await
        .expect("candidate ready");
    assert!(matches!(
        ownership.mark_draining(&old, &owner).await,
        Err(GatewayServiceOwnershipError::Conflict)
    ));
    assert_eq!(
        ownership
            .promote_ready(&candidate_ready, &owner)
            .await
            .expect("candidate cutover"),
        Some(fixture.revision)
    );
    ownership
        .mark_draining(&old, &owner)
        .await
        .expect("old revision drain after cutover");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_promotion_rejects_superseded_or_revoked_candidates() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let first_candidate = seed_service_revision(&pool, fixture.gateway).await;
    let second_candidate = seed_service_revision(&pool, fixture.gateway).await;
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway)
        .bind(first_candidate)
        .execute(&pool)
        .await
        .expect("first desired candidate");
    let ownership = worker_ownership().await;
    let owner = GatewayServiceOwner::new("supersede-host", Uuid::new_v4()).expect("owner");
    let first = ownership
        .claim_new(
            fixture.gateway,
            first_candidate,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("first candidate claim");
    let first = ownership
        .mark_ready(
            &ownership
                .mark_starting(&first, &owner)
                .await
                .expect("first candidate starting"),
            &owner,
        )
        .await
        .expect("first candidate ready");
    sqlx::query("UPDATE gateways SET lifecycle = 'paused' WHERE id = $1")
        .bind(fixture.gateway)
        .execute(&pool)
        .await
        .expect("pause gateway");
    assert!(matches!(
        ownership.promote_ready(&first, &owner).await,
        Err(GatewayServiceOwnershipError::Conflict)
    ));
    sqlx::query("UPDATE gateways SET lifecycle = 'enabled' WHERE id = $1")
        .bind(fixture.gateway)
        .execute(&pool)
        .await
        .expect("enable gateway");
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway)
        .bind(second_candidate)
        .execute(&pool)
        .await
        .expect("superseding desired candidate");
    let before_superseded_events: i64 = gateway_event_count(&pool, fixture.gateway).await;
    assert!(matches!(
        ownership.promote_ready(&first, &owner).await,
        Err(GatewayServiceOwnershipError::Conflict)
    ));
    assert_eq!(
        gateway_event_count(&pool, fixture.gateway).await,
        before_superseded_events
    );
    assert_eq!(
        active_pointer(&pool, fixture.gateway).await,
        Some(fixture.revision)
    );

    let second = ownership
        .claim_new(
            fixture.gateway,
            second_candidate,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("second candidate claim");
    let second = ownership
        .mark_ready(
            &ownership
                .mark_starting(&second, &owner)
                .await
                .expect("second candidate starting"),
            &owner,
        )
        .await
        .expect("second candidate ready");
    let release: Uuid =
        sqlx::query_scalar("SELECT release_id FROM gateway_revisions WHERE id = $1")
            .bind(second_candidate)
            .fetch_one(&pool)
            .await
            .expect("candidate release");
    sqlx::query("UPDATE releases SET state = 'revoked', revoked_at = now() WHERE id = $1")
        .bind(release)
        .execute(&pool)
        .await
        .expect("revoke candidate release");
    let before_revoked_events: i64 = gateway_event_count(&pool, fixture.gateway).await;
    assert!(matches!(
        ownership.promote_ready(&second, &owner).await,
        Err(GatewayServiceOwnershipError::Conflict)
    ));
    assert_eq!(
        gateway_event_count(&pool, fixture.gateway).await,
        before_revoked_events
    );
    assert_eq!(
        active_pointer(&pool, fixture.gateway).await,
        Some(fixture.revision)
    );
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_promotion_rechecks_expiry_after_instance_lock() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let candidate = seed_service_revision(&pool, fixture.gateway).await;
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway)
        .bind(candidate)
        .execute(&pool)
        .await
        .expect("desired candidate");
    let ownership = worker_ownership().await;
    let owner = GatewayServiceOwner::new("promotion-lock-host", Uuid::new_v4()).expect("owner");
    let claim = ownership
        .claim_new(fixture.gateway, candidate, &owner, Duration::from_secs(1))
        .await
        .expect("candidate claim");
    let ready = ownership
        .mark_ready(
            &ownership
                .mark_starting(&claim, &owner)
                .await
                .expect("candidate starting"),
            &owner,
        )
        .await
        .expect("candidate ready");
    let mut blocker = pool.begin().await.expect("blocker transaction");
    sqlx::query("SELECT id FROM gateway_service_instances WHERE id = $1 FOR UPDATE")
        .bind(ready.identity.instance_id)
        .execute(&mut *blocker)
        .await
        .expect("lock candidate instance");
    let promotion = tokio::spawn({
        let ownership = ownership.clone();
        let ready = ready.clone();
        let owner = owner.clone();
        async move { ownership.promote_ready(&ready, &owner).await }
    });
    wait_for_lock(&pool, "gateway_service_instances").await;
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    blocker.commit().await.expect("release candidate lock");
    assert!(matches!(
        promotion.await.expect("promotion task"),
        Err(GatewayServiceOwnershipError::StaleLease)
    ));
    let active: Option<Uuid> =
        sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
            .bind(fixture.gateway)
            .fetch_one(&pool)
            .await
            .expect("active pointer");
    assert_eq!(active, Some(fixture.revision));
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_promotion_rechecks_expiry_after_release_lock() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let candidate = seed_service_revision(&pool, fixture.gateway).await;
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway)
        .bind(candidate)
        .execute(&pool)
        .await
        .expect("desired candidate");
    let release: Uuid =
        sqlx::query_scalar("SELECT release_id FROM gateway_revisions WHERE id = $1")
            .bind(candidate)
            .fetch_one(&pool)
            .await
            .expect("candidate release");
    let ownership = worker_ownership().await;
    let owner =
        GatewayServiceOwner::new("promotion-release-lock-host", Uuid::new_v4()).expect("owner");
    let claim = ownership
        .claim_new(fixture.gateway, candidate, &owner, Duration::from_secs(1))
        .await
        .expect("candidate claim");
    let ready = ownership
        .mark_ready(
            &ownership
                .mark_starting(&claim, &owner)
                .await
                .expect("candidate starting"),
            &owner,
        )
        .await
        .expect("candidate ready");
    let mut blocker = pool.begin().await.expect("release blocker transaction");
    sqlx::query("SELECT id FROM releases WHERE id = $1 FOR UPDATE")
        .bind(release)
        .execute(&mut *blocker)
        .await
        .expect("lock candidate release");
    let promotion = tokio::spawn({
        let ownership = ownership.clone();
        let ready = ready.clone();
        let owner = owner.clone();
        async move { ownership.promote_ready(&ready, &owner).await }
    });
    wait_for_lock(&pool, "releases AS release").await;
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    blocker.commit().await.expect("release candidate lock");
    assert!(matches!(
        promotion.await.expect("promotion task"),
        Err(GatewayServiceOwnershipError::StaleLease)
    ));
    assert_eq!(
        active_pointer(&pool, fixture.gateway).await,
        Some(fixture.revision)
    );
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn ownership_cleanup_failure_keeps_live_uniqueness_and_batch_is_bounded() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let ownership = worker_ownership().await;
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
    let ownership = worker_ownership().await;
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

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_failure_is_idempotent_and_backoff_survives_restart() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = worker_ownership().await;
    let failures = worker_failure_store().await;
    let owner = GatewayServiceOwner::new("failure-host", Uuid::new_v4()).expect("owner");
    let claim = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("claim");
    let startup = GatewayServiceFailure::new(GatewayServiceFailureCode::Startup, None, None)
        .expect("startup failure");
    let invalid_shape = sqlx::query(
        "UPDATE gateway_service_instances
            SET failure_code = NULL, failed_at = NULL, exit_code = 7
          WHERE id = $1",
    )
    .bind(claim.identity.instance_id)
    .execute(&pool)
    .await
    .expect_err("non-exit failure metadata shape must be rejected");
    assert!(
        invalid_shape
            .to_string()
            .contains("gateway_service_instance_exit_failure_check")
    );
    failures
        .record_failure(&claim, &owner, startup)
        .await
        .expect("record startup failure");
    let recorded: (String, i32, Option<time::OffsetDateTime>) = sqlx::query_as(
        "SELECT instance.failure_code, retry.failure_streak, retry.next_retry_at
           FROM gateway_service_instances AS instance
           JOIN gateway_service_retry_state AS retry
             ON retry.gateway_id = instance.gateway_id
            AND retry.revision_id = instance.revision_id
          WHERE instance.id = $1",
    )
    .bind(claim.identity.instance_id)
    .fetch_one(&pool)
    .await
    .expect("failure state");
    assert_eq!(recorded.0, "startup");
    assert_eq!(recorded.1, 1);
    assert!(recorded.2.is_some());
    assert!(
        sqlx::query(
            "UPDATE gateway_service_retry_state
            SET next_retry_at = NULL
          WHERE gateway_id = $1 AND revision_id = $2",
        )
        .bind(fixture.gateway)
        .bind(fixture.revision)
        .execute(&pool)
        .await
        .is_err()
    );

    let duplicate =
        GatewayServiceFailure::new(GatewayServiceFailureCode::UnexpectedExit, Some(17), None)
            .expect("duplicate failure");
    failures
        .record_failure(&claim, &owner, duplicate)
        .await
        .expect("duplicate failure is harmless");
    let unchanged: (String, i32) = sqlx::query_as(
        "SELECT instance.failure_code, retry.failure_streak
           FROM gateway_service_instances AS instance
           JOIN gateway_service_retry_state AS retry
             ON retry.gateway_id = instance.gateway_id
            AND retry.revision_id = instance.revision_id
          WHERE instance.id = $1",
    )
    .bind(claim.identity.instance_id)
    .fetch_one(&pool)
    .await
    .expect("unchanged failure state");
    assert_eq!(unchanged, (String::from("startup"), 1));
    assert!(matches!(
        ownership
            .claim_new(
                fixture.gateway,
                fixture.revision,
                &owner,
                Duration::from_secs(30),
            )
            .await,
        Err(GatewayServiceOwnershipError::Conflict)
    ));

    let stopping = ownership
        .mark_stopping(&claim, &owner)
        .await
        .expect("stop failed instance");
    ownership
        .mark_cleaned(&stopping, &owner)
        .await
        .expect("clean failed instance");
    let restarted_ownership = worker_ownership().await;
    assert!(matches!(
        restarted_ownership
            .claim_new(
                fixture.gateway,
                fixture.revision,
                &owner,
                Duration::from_secs(30),
            )
            .await,
        Err(GatewayServiceOwnershipError::Conflict)
    ));
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    let replacement = restarted_ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("claim after durable backoff");
    let starting = restarted_ownership
        .mark_starting(&replacement, &owner)
        .await
        .expect("starting replacement");
    let ready = restarted_ownership
        .mark_ready(&starting, &owner)
        .await
        .expect("ready replacement");
    let reset: (i32, Option<time::OffsetDateTime>) = sqlx::query_as(
        "SELECT failure_streak, next_retry_at
           FROM gateway_service_retry_state
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .fetch_one(&pool)
    .await
    .expect("reset retry state");
    assert_eq!(reset, (0, None));
    failures
        .record_failure(
            &ready,
            &owner,
            GatewayServiceFailure::new(GatewayServiceFailureCode::UnexpectedExit, Some(17), None)
                .expect("exit failure"),
        )
        .await
        .expect("record exit failure");
    let exit: (String, Option<i32>, Option<i32>) = sqlx::query_as(
        "SELECT failure_code, exit_code, exit_signal
           FROM gateway_service_instances WHERE id = $1",
    )
    .bind(ready.identity.instance_id)
    .fetch_one(&pool)
    .await
    .expect("stored exit failure");
    assert_eq!(exit, (String::from("unexpected_exit"), Some(17), None));
    let stopping = restarted_ownership
        .mark_stopping(&ready, &owner)
        .await
        .expect("stop replacement");
    restarted_ownership
        .mark_cleaned(&stopping, &owner)
        .await
        .expect("clean replacement");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_failure_requires_exact_live_fence_and_redacted_exit_shape() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = worker_ownership().await;
    let failures = worker_failure_store().await;
    let owner = GatewayServiceOwner::new("failure-fence-host", Uuid::new_v4()).expect("owner");
    let claim = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("claim");
    let mut wrong_fence = claim.clone();
    wrong_fence.fencing_token += 1;
    let failure = GatewayServiceFailure::new(GatewayServiceFailureCode::Startup, None, None)
        .expect("failure");
    assert!(matches!(
        failures.record_failure(&wrong_fence, &owner, failure).await,
        Err(GatewayServiceFailureStoreError::StaleLease)
    ));
    let wrong_owner =
        GatewayServiceOwner::new("failure-fence-host", Uuid::new_v4()).expect("owner");
    assert!(matches!(
        failures.record_failure(&claim, &wrong_owner, failure).await,
        Err(GatewayServiceFailureStoreError::StaleLease)
    ));
    let expired_fixture = seed_fixture(&pool, "http.service.v1").await;
    let expired_claim = ownership
        .claim_new(
            expired_fixture.gateway,
            expired_fixture.revision,
            &owner,
            Duration::from_millis(5),
        )
        .await
        .expect("short-lived claim");
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(matches!(
        failures
            .record_failure(&expired_claim, &owner, failure)
            .await,
        Err(GatewayServiceFailureStoreError::StaleLease)
    ));
    assert!(
        GatewayServiceFailure::new(GatewayServiceFailureCode::Startup, Some(1), None,).is_err()
    );
    assert!(
        GatewayServiceFailure::new(GatewayServiceFailureCode::UnexpectedExit, Some(256), None,)
            .is_err()
    );
    let signal =
        GatewayServiceFailure::new(GatewayServiceFailureCode::UnexpectedExit, None, Some(9))
            .expect("bounded signal");
    assert_eq!(signal.exit_signal, Some(9));
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_failure_backoff_is_scoped_to_one_revision() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let candidate = seed_service_revision(&pool, fixture.gateway).await;
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway)
        .bind(candidate)
        .execute(&pool)
        .await
        .expect("desired candidate");
    let ownership = worker_ownership().await;
    let failures = worker_failure_store().await;
    let owner = GatewayServiceOwner::new("failure-revision-host", Uuid::new_v4()).expect("owner");
    let old = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("old claim");
    let candidate_claim = ownership
        .claim_new(fixture.gateway, candidate, &owner, Duration::from_secs(30))
        .await
        .expect("candidate claim");
    failures
        .record_failure(
            &old,
            &owner,
            GatewayServiceFailure::new(GatewayServiceFailureCode::Health, None, None)
                .expect("health failure"),
        )
        .await
        .expect("record old revision failure");
    let old_stopping = ownership
        .mark_stopping(&old, &owner)
        .await
        .expect("stop old revision");
    ownership
        .mark_cleaned(&old_stopping, &owner)
        .await
        .expect("clean old revision");
    assert!(matches!(
        ownership
            .claim_new(
                fixture.gateway,
                fixture.revision,
                &owner,
                Duration::from_secs(30),
            )
            .await,
        Err(GatewayServiceOwnershipError::Conflict)
    ));
    let candidate_stopping = ownership
        .mark_stopping(&candidate_claim, &owner)
        .await
        .expect("stop candidate");
    ownership
        .mark_cleaned(&candidate_stopping, &owner)
        .await
        .expect("clean candidate");
    let replacement = ownership
        .claim_new(fixture.gateway, candidate, &owner, Duration::from_secs(30))
        .await
        .expect("candidate revision ignores old backoff");
    let replacement_stopping = ownership
        .mark_stopping(&replacement, &owner)
        .await
        .expect("stop candidate replacement");
    ownership
        .mark_cleaned(&replacement_stopping, &owner)
        .await
        .expect("clean candidate replacement");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_failure_and_ready_reset_recheck_expiry_after_retry_lock() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = worker_ownership().await;
    let failures = worker_failure_store().await;
    let owner = GatewayServiceOwner::new("failure-lock-host", Uuid::new_v4()).expect("owner");
    let claim = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &owner,
            Duration::from_secs(1),
        )
        .await
        .expect("claim");
    let startup = GatewayServiceFailure::new(GatewayServiceFailureCode::Startup, None, None)
        .expect("startup failure");
    failures
        .record_failure(&claim, &owner, startup)
        .await
        .expect("initial failure");
    let mut blocker = pool.begin().await.expect("retry blocker");
    sqlx::query(
        "SELECT gateway_id
           FROM gateway_service_retry_state
          WHERE gateway_id = $1 AND revision_id = $2
          FOR UPDATE",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .execute(&mut *blocker)
    .await
    .expect("lock retry state");
    let duplicate_task = {
        let failures = failures.clone();
        let claim = claim.clone();
        let owner = owner.clone();
        tokio::spawn(async move { failures.record_failure(&claim, &owner, startup).await })
    };
    wait_for_lock_named(&pool, "gateway-failure-test", "gateway_service_retry_state").await;
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    blocker.commit().await.expect("release retry blocker");
    assert!(matches!(
        duplicate_task.await.expect("duplicate task"),
        Err(GatewayServiceFailureStoreError::StaleLease)
    ));
    let unchanged: (String, i32) = sqlx::query_as(
        "SELECT instance.failure_code, retry.failure_streak
           FROM gateway_service_instances AS instance
           JOIN gateway_service_retry_state AS retry
             ON retry.gateway_id = instance.gateway_id
            AND retry.revision_id = instance.revision_id
          WHERE instance.id = $1",
    )
    .bind(claim.identity.instance_id)
    .fetch_one(&pool)
    .await
    .expect("unchanged expired failure");
    assert_eq!(unchanged, (String::from("startup"), 1));

    let second = seed_fixture(&pool, "http.service.v1").await;
    let second_claim = ownership
        .claim_new(
            second.gateway,
            second.revision,
            &owner,
            Duration::from_secs(1),
        )
        .await
        .expect("second claim");
    let second_starting = ownership
        .mark_starting(&second_claim, &owner)
        .await
        .expect("second starting");
    failures
        .record_failure(&second_starting, &owner, startup)
        .await
        .expect("second initial failure");
    let mut reset_blocker = pool.begin().await.expect("reset blocker");
    sqlx::query(
        "SELECT gateway_id
           FROM gateway_service_retry_state
          WHERE gateway_id = $1 AND revision_id = $2
          FOR UPDATE",
    )
    .bind(second.gateway)
    .bind(second.revision)
    .execute(&mut *reset_blocker)
    .await
    .expect("lock reset retry state");
    let reset_task = {
        let ownership = ownership.clone();
        let second_starting = second_starting.clone();
        let owner = owner.clone();
        tokio::spawn(async move { ownership.mark_ready(&second_starting, &owner).await })
    };
    wait_for_lock_named(
        &pool,
        "gateway-ownership-test",
        "gateway_service_retry_state",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    reset_blocker.commit().await.expect("release reset blocker");
    assert!(matches!(
        reset_task.await.expect("reset task"),
        Err(GatewayServiceOwnershipError::StaleLease)
    ));
    let state: (String, String, i32) = sqlx::query_as(
        "SELECT instance.state, instance.failure_code, retry.failure_streak
           FROM gateway_service_instances AS instance
           JOIN gateway_service_retry_state AS retry
             ON retry.gateway_id = instance.gateway_id
            AND retry.revision_id = instance.revision_id
          WHERE instance.id = $1",
    )
    .bind(second_claim.identity.instance_id)
    .fetch_one(&pool)
    .await
    .expect("rolled back reset");
    assert_eq!(
        state,
        (String::from("starting"), String::from("startup"), 1)
    );
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
    assert!(max_version >= 76);
    println!("REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration={max_version}");
    Some(pool)
}

async fn worker_ownership() -> PostgresGatewayServiceOwnership {
    let database_url =
        env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("worker ownership test database URL");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SET application_name = 'gateway-ownership-test'")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect worker PostgreSQL pool");
    PostgresGatewayServiceOwnership::new(pool)
}

async fn worker_failure_store() -> PostgresGatewayServiceFailureStore {
    let database_url =
        env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("worker failure test database URL");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SET application_name = 'gateway-failure-test'")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect worker failure PostgreSQL pool");
    PostgresGatewayServiceFailureStore::new(pool)
}

async fn wait_for_lock(pool: &sqlx::PgPool, query_fragment: &str) {
    wait_for_lock_named(pool, "gateway-ownership-test", query_fragment).await;
}

async fn wait_for_lock_named(pool: &sqlx::PgPool, application_name: &str, query_fragment: &str) {
    let pattern = format!("%{query_fragment}%");
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                     SELECT 1 FROM pg_stat_activity
                      WHERE application_name = $2
                        AND state = 'active'
                        AND wait_event_type = 'Lock'
                        AND query LIKE $1
                 )",
            )
            .bind(&pattern)
            .bind(application_name)
            .fetch_one(pool)
            .await
            .expect("inspect PostgreSQL lock wait");
            if waiting {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("ownership operation reached expected lock wait");
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
    let (repository, owner): (Uuid, Uuid) =
        sqlx::query_as("SELECT repository_id, created_by FROM gateways WHERE id = $1")
            .bind(gateway)
            .fetch_one(pool)
            .await
            .expect("gateway coordinates");
    let release = insert_published_release(pool, repository, owner).await;
    let mut revision_hash = [8_u8; 32];
    revision_hash[..16].copy_from_slice(revision.as_bytes());
    let query = "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, release_id,
             release_agent_id, release_agent_key, handler_contract, exposure,
             parameters, secret_slots, service_loopback_port,
             service_readiness_path, service_health_path, normalized_hash,
             created_by)
         SELECT $2, gateway.id, gateway.project_id, gateway.repository_id, $3,
                (SELECT id FROM release_agents WHERE release_id = $3 LIMIT 1),
                'ownership-service', 'http.service.v1', 'public', '{}',
                '{hook}', 18081, '/ready', '/health', $4, gateway.created_by
           FROM gateways AS gateway
          WHERE gateway.id = $1";
    let result = sqlx::query(query)
        .bind(gateway)
        .bind(revision)
        .bind(release)
        .bind(revision_hash.as_slice())
        .execute(pool)
        .await
        .expect("replacement service revision");
    assert_eq!(result.rows_affected(), 1);
    revision
}

async fn gateway_event_count(pool: &sqlx::PgPool, gateway: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM application_events
          WHERE aggregate_type = 'gateway' AND aggregate_id = $1",
    )
    .bind(gateway)
    .fetch_one(pool)
    .await
    .expect("gateway event count")
}

async fn active_pointer(pool: &sqlx::PgPool, gateway: Uuid) -> Option<Uuid> {
    sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
        .bind(gateway)
        .fetch_one(pool)
        .await
        .expect("active gateway pointer")
}

async fn insert_published_release(pool: &sqlx::PgPool, repository: Uuid, owner: Uuid) -> Uuid {
    let release = Uuid::new_v4();
    let build = Uuid::new_v4();
    let source_commit = format!("00000000{}", release.simple());
    sqlx::query(
        "INSERT INTO build_requests
            (id, repository_id, source_commit, source_ref,
             build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)",
    )
    .bind(build)
    .bind(repository)
    .bind(&source_commit)
    .bind([4_u8; 32].as_slice())
    .bind(owner)
    .execute(pool)
    .await
    .expect("build request");
    sqlx::query(
        "INSERT INTO releases
            (id, repository_id, version, source_commit, source_ref,
             build_request_id, build_definition_hash, configuration,
             configuration_hash, manifest_hash, state, publication_actor_id,
             published_at)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}', $7, $8,
                 'published', $9, now())",
    )
    .bind(release)
    .bind(repository)
    .bind(format!("ownership-release-{release}"))
    .bind(&source_commit)
    .bind(build)
    .bind([4_u8; 32].as_slice())
    .bind([5_u8; 32].as_slice())
    .bind([6_u8; 32].as_slice())
    .bind(owner)
    .execute(pool)
    .await
    .expect("published release");
    let family = Uuid::new_v4();
    let agent = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, $3)",
    )
    .bind(family)
    .bind(repository)
    .bind(format!("ownership-agent-{release}"))
    .execute(pool)
    .await
    .expect("agent family");
    sqlx::query(
        "INSERT INTO release_agents
            (id, release_id, family_id, agent_key, display_name,
             runtime_contract, runtime_contract_hash, parameter_schema,
             secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, 'ownership-service', 'Ownership service', '{}',
                 $4, '[]', '[]', false)",
    )
    .bind(agent)
    .bind(release)
    .bind(family)
    .bind([7_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("release agent");
    release
}
