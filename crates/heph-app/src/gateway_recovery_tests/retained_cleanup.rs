use super::*;

#[path = "retained_cleanup_phase.rs"]
mod retained_cleanup_phase;
use retained_cleanup_phase::{RetainedCleanupContext, complete_retained_cleanup_case};

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_retries_retained_cleanup_before_admitting_next_revision() {
    daemon_loop_retries_retained_cleanup_case(false).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_recovers_expired_owned_cleanup_before_admitting_next_revision() {
    daemon_loop_retries_retained_cleanup_case(true).await;
}

// This integration case intentionally keeps the complete retained-VM lifecycle
// together so both the normal and expired-lease variants share identical setup.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
async fn daemon_loop_retries_retained_cleanup_case(expire_before_retry: bool) {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let desired_revision = seed_service_candidate(&pool, fixture).await;
    let third_revision = seed_service_candidate(&pool, fixture).await;
    let observing_targets = Arc::new(ObservingTargets::new(
        database.worker.clone(),
        third_revision,
    ));
    let destroy_gate = Arc::new(DestroyGate::new());
    if expire_before_retry {
        destroy_gate.hold_first_failure();
    }
    let cleanup_calls = Arc::new(AtomicUsize::new(0));
    let resolver = Arc::new(RecordingLaunchResolver {
        cleanup_calls: Arc::clone(&cleanup_calls),
    });
    let ownership_override = expire_before_retry.then(|| {
        Arc::new(
            ObservingOwnership::new(database.worker.clone())
                .with_renewal_gate(destroy_gate.clone()),
        ) as Arc<dyn GatewayServiceOwnership>
    });
    let (task, cancellation, caddy_started, caddy_release, _destroyed, provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            fixture,
            true,
            false,
            Some(desired_revision),
            None,
            ownership_override,
            Some(observing_targets.clone() as Arc<dyn GatewayServiceTargetStore>),
            Some(destroy_gate.clone()),
            Some(resolver),
            None,
        )
        .await;
    tokio::time::timeout(StdDuration::from_secs(10), caddy_started.notified())
        .await
        .expect("Caddy remains blocked while cleanup retry runs");
    assert!(wait_for_ready(&pool, fixture).await);

    let active_instance: (Uuid, i64, String, String, Uuid) = sqlx::query_as(
        "SELECT id, fencing_token, vm_id, owner_host_id, owner_uuid
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .fetch_one(&pool)
    .await
    .expect("read original service instance");
    destroy_gate.arm_first_failure_for(VmId(active_instance.2.clone()));
    let invocation = insert_invocation(
        &pool,
        Fixture {
            service_instance: Some(active_instance.0),
            ..fixture
        },
        OffsetDateTime::now_utc(),
    )
    .await;
    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let active: Option<Uuid> =
                sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
                    .bind(fixture.gateway)
                    .fetch_one(&pool)
                    .await
                    .expect("read desired promotion");
            if active == Some(desired_revision)
                && wait_for_ready(
                    &pool,
                    Fixture {
                        revision: desired_revision,
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
    .expect("replacement is ready");
    let (replacement_instance, replacement_vm_id): (Uuid, String) = sqlx::query_as(
        "SELECT id, vm_id
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'",
    )
    .bind(fixture.gateway)
    .bind(desired_revision)
    .fetch_one(&pool)
    .await
    .expect("read replacement service instance");
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway)
        .bind(third_revision)
        .execute(&pool)
        .await
        .expect("declare next candidate");
    sqlx::query("SELECT gateway_invocation_complete($1, 'completed')")
        .bind(invocation)
        .fetch_one(&pool)
        .await
        .expect("complete accepted invocation");
    sqlx::query("DELETE FROM gateway_invocations WHERE id = $1")
        .bind(invocation)
        .execute(&pool)
        .await
        .expect("remove completed invocation");
    tokio::time::timeout(StdDuration::from_secs(10), destroy_gate.entered.notified())
        .await
        .expect("first physical cleanup attempt fails");
    assert_eq!(destroy_gate.attempts.load(Ordering::Acquire), 1);
    assert_eq!(
        destroy_gate
            .attempt_ids
            .lock()
            .expect("destroy attempt ids")
            .first()
            .map(|id| id.0.as_str()),
        Some(active_instance.2.as_str()),
    );
    if expire_before_retry {
        tokio::time::timeout(
            StdDuration::from_secs(5),
            destroy_gate.renewal_pause_entered().notified(),
        )
        .await
        .expect("A renewal pauses before database access");
        // The ownership wrapper pauses only A's renewal before it can acquire
        // the gateway lock, so B remains independently renewable.
        // Wait for the durable row to expire while the first failure is held.
        tokio::time::timeout(StdDuration::from_secs(5), async {
            loop {
                let expired: bool = sqlx::query_scalar(
                    "SELECT clock_timestamp() >= lease_expires_at
                       FROM gateway_service_instances
                      WHERE id = $1",
                )
                .bind(active_instance.0)
                .fetch_one(&pool)
                .await
                .expect("observe retained cleanup lease expiry");
                if expired {
                    break;
                }
                tokio::time::sleep(StdDuration::from_millis(20)).await;
            }
        })
        .await
        .expect("retained cleanup lease expires while A renewal is paused");
        let (replacement_state, replacement_lease_live): (String, bool) = sqlx::query_as(
            "SELECT state, lease_expires_at > clock_timestamp()
               FROM gateway_service_instances
              WHERE id = $1",
        )
        .bind(replacement_instance)
        .fetch_one(&pool)
        .await
        .expect("read healthy B state after A expiry");
        assert_eq!(replacement_state, "ready");
        assert!(
            replacement_lease_live,
            "B lease must remain live while A expires"
        );
        let active_revision: Uuid = sqlx::query_scalar(
            "SELECT active_revision_id
               FROM gateways
              WHERE id = $1",
        )
        .bind(fixture.gateway)
        .fetch_one(&pool)
        .await
        .expect("read active B revision after A expiry");
        assert_eq!(active_revision, desired_revision);
        destroy_gate.release_first_failure();
        destroy_gate.release_renewal_pause();
    }
    complete_retained_cleanup_case(
        RetainedCleanupContext {
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
        },
        expire_before_retry,
    )
    .await;
}
