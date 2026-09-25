use super::*;

pub(super) struct RetainedCleanupContext {
    pub(super) database: IsolatedStartupDatabase,
    pub(super) pool: sqlx::PgPool,
    pub(super) fixture: Fixture,
    pub(super) third_revision: Uuid,
    pub(super) observing_targets: Arc<ObservingTargets>,
    pub(super) destroy_gate: Arc<DestroyGate>,
    pub(super) cleanup_calls: Arc<AtomicUsize>,
    pub(super) task: tokio::task::JoinHandle<()>,
    pub(super) cancellation: CancellationToken,
    pub(super) caddy_release: Arc<tokio::sync::Notify>,
    pub(super) provisioned: Arc<AtomicUsize>,
    pub(super) active_instance: (Uuid, i64, String, String, Uuid),
    pub(super) replacement_instance: Uuid,
    pub(super) replacement_vm_id: String,
}

#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub(super) async fn complete_retained_cleanup_case(
    context: RetainedCleanupContext,
    expire_before_retry: bool,
) {
    let RetainedCleanupContext {
        database,
        pool,
        fixture,
        third_revision,
        observing_targets,
        destroy_gate,
        cleanup_calls,
        task,
        cancellation,
        caddy_release,
        provisioned,
        active_instance,
        replacement_instance,
        replacement_vm_id,
    } = context;
    tokio::time::timeout(StdDuration::from_secs(10), destroy_gate.entered.notified())
        .await
        .expect("automatic retry reaches the controlled physical cleanup barrier");
    let retry_ids = destroy_gate
        .attempt_ids
        .lock()
        .expect("destroy attempt ids")
        .clone();
    let active_vm_id = VmId(active_instance.2.clone());
    let active_attempts = retry_ids.iter().filter(|id| *id == &active_vm_id).count();
    assert!(
        active_attempts >= 2,
        "retained A cleanup must retry its exact VM; attempts={retry_ids:?}",
    );
    assert!(
        retry_ids
            .iter()
            .all(|id| id.0.as_str() != replacement_vm_id.as_str()),
        "healthy B cleanup must not be attempted while A is held; attempts={retry_ids:?}",
    );

    let retained_instance: (Uuid, i64, String, String) = sqlx::query_as(
        "SELECT id, fencing_token, vm_id, state
           FROM gateway_service_instances
          WHERE id = $1",
    )
    .bind(active_instance.0)
    .fetch_one(&pool)
    .await
    .expect("read retained original service claim");
    assert_eq!(retained_instance.0, active_instance.0);
    if expire_before_retry {
        assert_eq!(retained_instance.1, active_instance.1 + 1);
        let retained_owner: (String, Uuid) = sqlx::query_as(
            "SELECT owner_host_id, owner_uuid
               FROM gateway_service_instances
              WHERE id = $1",
        )
        .bind(active_instance.0)
        .fetch_one(&pool)
        .await
        .expect("read recovered cleanup owner");
        assert_eq!(retained_owner.0, active_instance.3);
        assert_eq!(retained_owner.1, active_instance.4);
    } else {
        assert_eq!(retained_instance.1, active_instance.1);
    }
    assert_eq!(retained_instance.2, active_instance.2);
    assert_eq!(retained_instance.3, "stopping");
    assert!(
        destroy_gate
            .orphan_attempt_ids
            .lock()
            .expect("orphan attempt ids")
            .is_empty(),
        "retained VM cleanup must never use the orphan path",
    );
    let scans_at_retry = observing_targets.observed_scans.load(Ordering::Acquire);
    let cleanup_heartbeat_before_retry: OffsetDateTime = sqlx::query_scalar(
        "SELECT heartbeat_at
           FROM gateway_service_instances
          WHERE id = $1",
    )
    .bind(active_instance.0)
    .fetch_one(&pool)
    .await
    .expect("read retained cleanup heartbeat at retry barrier");
    let heartbeat_before_retry: OffsetDateTime = sqlx::query_scalar(
        "SELECT heartbeat_at
           FROM gateway_service_instances
          WHERE id = $1",
    )
    .bind(replacement_instance)
    .fetch_one(&pool)
    .await
    .expect("read replacement heartbeat at retry barrier");

    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let heartbeat: OffsetDateTime = sqlx::query_scalar(
                "SELECT heartbeat_at FROM gateway_service_instances WHERE id = $1",
            )
            .bind(replacement_instance)
            .fetch_one(&pool)
            .await
            .expect("read replacement heartbeat during retry");
            let cleanup_heartbeat: OffsetDateTime = sqlx::query_scalar(
                "SELECT heartbeat_at FROM gateway_service_instances WHERE id = $1",
            )
            .bind(active_instance.0)
            .fetch_one(&pool)
            .await
            .expect("read retained cleanup heartbeat during retry");
            if heartbeat > heartbeat_before_retry
                && cleanup_heartbeat > cleanup_heartbeat_before_retry
                && observing_targets.observed_scans.load(Ordering::Acquire) >= scans_at_retry + 2
            {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("Caddy-blocked loop continues scans during retained cleanup");
    let third_instances: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(third_revision)
    .fetch_one(&pool)
    .await
    .expect("read candidate while cleanup retained");
    assert_eq!(third_instances, 0);
    assert_eq!(provisioned.load(Ordering::Acquire), 2);

    destroy_gate.release.notify_one();
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let state: String =
                sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
                    .bind(active_instance.0)
                    .fetch_one(&pool)
                    .await
                    .expect("read cleaned original service");
            if state == "cleaned" {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("retry completes durable cleanup");
    assert!(cleanup_calls.load(Ordering::Acquire) > 0);
    let cleaned_instance: (Uuid, i64, String, String, OffsetDateTime) = sqlx::query_as(
        "SELECT id, fencing_token, vm_id, state, cleaned_at
           FROM gateway_service_instances
          WHERE id = $1",
    )
    .bind(active_instance.0)
    .fetch_one(&pool)
    .await
    .expect("read cleaned original service identity");
    assert_eq!(cleaned_instance.0, active_instance.0);
    if expire_before_retry {
        assert_eq!(cleaned_instance.1, active_instance.1 + 1);
    } else {
        assert_eq!(cleaned_instance.1, active_instance.1);
    }
    assert_eq!(cleaned_instance.2, active_instance.2);
    assert_eq!(cleaned_instance.3, "cleaned");
    let cleaned_at = cleaned_instance.4;
    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let active: Option<Uuid> =
                sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
                    .bind(fixture.gateway)
                    .fetch_one(&pool)
                    .await
                    .expect("read next active revision");
            if active == Some(third_revision)
                && wait_for_ready(
                    &pool,
                    Fixture {
                        revision: third_revision,
                        ..fixture
                    },
                )
                .await
            {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("next candidate admitted after durable cleanup");
    let candidate_created_at: OffsetDateTime = sqlx::query_scalar(
        "SELECT min(created_at)
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(third_revision)
    .fetch_one(&pool)
    .await
    .expect("read candidate creation time after cleanup");
    assert!(
        candidate_created_at >= cleaned_at,
        "candidate must be created after original instance is durably cleaned",
    );
    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let state: String =
                sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
                    .bind(replacement_instance)
                    .fetch_one(&pool)
                    .await
                    .expect("read cleaned replacement service");
            if state == "cleaned" {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("replacement service cleans after next candidate becomes active");
    let replacement_destroyed = destroy_gate
        .attempt_ids
        .lock()
        .expect("destroy attempt ids")
        .iter()
        .any(|id| id.0 == replacement_vm_id);
    assert!(
        replacement_destroyed,
        "replacement VM must reach physical cleanup after candidate promotion",
    );

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("cleanup retry loop joins")
        .expect("cleanup retry loop task");
    caddy_release.notify_one();
    cleanup_startup_fixture(&pool, fixture).await;
    drop_isolated_startup_database(database).await;
}
