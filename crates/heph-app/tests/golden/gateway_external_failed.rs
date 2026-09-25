use super::*;

pub async fn exercise_external_gateway_service_unclean_restart(
    request: ExternalGatewayServiceUncleanRestart<'_>,
) {
    let ExternalGatewayServiceUncleanRestart {
        pool,
        fixture,
        gateway,
        app_config,
        root,
        root_image,
        daemon,
        first_instance_id,
        old_paths,
        first_proof,
        public_url,
    } = request;
    let old_ownership = read_gateway_service_ownership(pool, first_instance_id).await;
    let db_now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(pool)
        .await
        .expect("read database clock before daemon crash");
    assert!(
        old_ownership.3 > db_now,
        "service lease must be live before the daemon crash"
    );
    let exit_status = daemon.unclean_kill().await;
    assert_eq!(
        std::os::unix::process::ExitStatusExt::signal(&exit_status),
        Some(9),
        "external daemon must be terminated by SIGKILL"
    );
    eprintln!(
        "persistent-service-unclean-daemon-kill old_instance={first_instance_id} old_fence={} old_lease_expires_at={}",
        old_ownership.2, old_ownership.3
    );
    let old_resources_after_kill = (
        old_paths.0.exists(),
        old_paths.1.exists(),
        old_paths.2.exists(),
    );
    eprintln!(
        "persistent-service-unclean-daemon-residual-resources runtime={} cgroup={} materializer={}",
        old_resources_after_kill.0, old_resources_after_kill.1, old_resources_after_kill.2
    );
    let recovery_started_at: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(pool)
        .await
        .expect("read database clock before daemon restart");
    let restart_log = root.join("external-hephaestusd-restart.log");
    let mut restarted =
        spawn_external_golden_daemon(gateway, app_config, root, root_image, Some(&restart_log))
            .await;
    wait_for_external_daemon_health(&mut restarted).await;
    let replacement_instance_id = wait_for_gateway_service_boot_replacement(
        pool,
        fixture,
        first_instance_id,
        old_ownership,
        old_paths,
        recovery_started_at,
    )
    .await;
    let replacement_paths = gateway_service_resource_paths(replacement_instance_id);
    let replacement_proof = exercise_gateway_service_requests(public_url).await;
    assert_ne!(
        replacement_instance_id, first_instance_id,
        "unclean daemon restart must claim a fresh service instance"
    );
    assert_ne!(
        replacement_proof.startup_id, first_proof.startup_id,
        "unclean daemon restart must start a fresh guest process"
    );
    assert!(
        replacement_paths.0.is_dir(),
        "replacement VM runtime must exist before final shutdown"
    );
    assert!(
        replacement_paths.1.is_dir(),
        "replacement cgroup must exist before final shutdown"
    );
    assert!(
        replacement_paths.2.is_dir(),
        "replacement materializer must exist before final shutdown"
    );
    eprintln!(
        "persistent-service-unclean-daemon-recovered old_instance={first_instance_id} replacement_instance={replacement_instance_id} replacement_startup_id={}",
        replacement_proof.startup_id
    );
    restarted.graceful_shutdown().await;
    wait_for_gateway_service_cleaned(pool, fixture, replacement_instance_id).await;
    assert!(!replacement_paths.0.exists());
    assert!(!replacement_paths.1.exists());
    assert!(!replacement_paths.2.exists());
    eprintln!("persistent-service-unclean-daemon-recovery-passed");
}

/// Exercises a real failed candidate while the original ready revision keeps
/// serving public identity requests. The `/crash` readiness probe returns the
/// fixture's 503 before its process exits with 42; this acceptance requires
/// the durable unexpected-exit report so a generic startup failure cannot be
/// mistaken for guest execution.
#[allow(
    // This one acceptance path keeps failure, public continuity, and cleanup
    // assertions together so their ordering remains explicit.
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub async fn exercise_external_gateway_service_failed_candidate(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    daemon: ExternalGoldenDaemon,
    old_instance_id: uuid::Uuid,
    old_paths: (PathBuf, PathBuf, PathBuf),
    first_proof: &GatewayServiceRequestProof,
    public_url: &str,
    release_artifact_root: &Path,
) {
    let old_fencing_token: i64 = sqlx::query_scalar(
        "SELECT fencing_token
           FROM gateway_service_instances
          WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
    )
    .bind(old_instance_id)
    .bind(fixture.gateway_id)
    .bind(fixture.revision_id)
    .fetch_one(pool)
    .await
    .expect("read A fencing token before failed candidate");
    assert!(old_paths.0.is_dir() && old_paths.1.is_dir() && old_paths.2.is_dir());

    let candidate =
        seed_gateway_service_cutover_candidate(pool, fixture, release_artifact_root, "/crash")
            .await;
    let gateway_events_before_declaration: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM application_events
          WHERE aggregate_type = 'gateway' AND aggregate_id = $1",
    )
    .bind(fixture.gateway_id)
    .fetch_one(pool)
    .await
    .expect("count gateway events before failed candidate declaration");
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway_id)
        .bind(candidate.revision_id)
        .execute(pool)
        .await
        .expect("declare failed published candidate");
    let gateway_events_after_declaration: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM application_events
          WHERE aggregate_type = 'gateway' AND aggregate_id = $1",
    )
    .bind(fixture.gateway_id)
    .fetch_one(pool)
    .await
    .expect("count gateway events after failed candidate declaration");
    assert_eq!(
        gateway_events_after_declaration,
        gateway_events_before_declaration + 1,
        "declaring failed B emits exactly one durable gateway event"
    );

    // A bounded request after declaration confirms that the existing public
    // revision remains the one served while B is being attempted.
    let during_startup = exercise_gateway_service_requests(public_url).await;
    assert_eq!(during_startup.startup_id, first_proof.startup_id);
    let evidence = wait_for_failed_gateway_service_candidate(
        pool,
        fixture.gateway_id,
        candidate.revision_id,
        (old_instance_id, fixture.revision_id, old_fencing_token),
        first_proof,
        public_url,
    )
    .await;

    assert_eq!(evidence.gateway_id, fixture.gateway_id);
    assert_eq!(evidence.revision_id, candidate.revision_id);
    assert!(
        !evidence.observed_ready,
        "failed B must not be observed Ready during lifecycle polling"
    );
    assert!(
        !evidence.observed_promoted,
        "failed B must not be observed as the active revision during polling"
    );
    assert_eq!(evidence.state, "cleaned");
    let failure_code = evidence
        .failure_code
        .as_deref()
        .expect("failed B retains a durable failure classification");
    assert_eq!(
        failure_code, "unexpected_exit",
        "failed-candidate proof must observe the guest crash, not startup failure"
    );
    assert_eq!(evidence.exit_code, Some(42));
    assert!(evidence.exit_signal.is_none());
    assert!(evidence.failed_at.is_some());
    assert_eq!(evidence.fencing_token, evidence.initial_fencing_token);

    let b_invocations: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM gateway_invocations
          WHERE gateway_id = $1
            AND gateway_revision_id = $2",
    )
    .bind(fixture.gateway_id)
    .bind(candidate.revision_id)
    .fetch_one(pool)
    .await
    .expect("count all failed B invocations");
    assert_eq!(
        b_invocations, 0,
        "failed B must not receive invocations of any outcome"
    );
    let b_bound_invocations: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM gateway_invocations
          WHERE gateway_id = $1
            AND gateway_revision_id = $2
            AND service_instance_id = $3
            AND service_instance_fencing_token = $4",
    )
    .bind(fixture.gateway_id)
    .bind(candidate.revision_id)
    .bind(evidence.instance_id)
    .bind(evidence.fencing_token)
    .fetch_one(pool)
    .await
    .expect("count failed B invocations bound to its exact lease");
    assert_eq!(
        b_bound_invocations, 0,
        "failed B must not receive invocations on its exact instance and fence"
    );

    let gateway_events_after_cleanup: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM application_events
          WHERE aggregate_type = 'gateway' AND aggregate_id = $1",
    )
    .bind(fixture.gateway_id)
    .fetch_one(pool)
    .await
    .expect("count gateway events after failed candidate cleanup");
    assert_eq!(
        gateway_events_after_cleanup, gateway_events_after_declaration,
        "failed B cleanup must not promote or roll back a gateway pointer"
    );

    let (active_revision, old_state): (Option<uuid::Uuid>, String) = sqlx::query_as(
        "SELECT gateway.active_revision_id, instance.state
           FROM gateways AS gateway
           JOIN gateway_service_instances AS instance
             ON instance.id = $2
            AND instance.gateway_id = gateway.id
            AND instance.revision_id = $3
          WHERE gateway.id = $1",
    )
    .bind(fixture.gateway_id)
    .bind(old_instance_id)
    .bind(fixture.revision_id)
    .fetch_one(pool)
    .await
    .expect("read active A after failed B cleanup");
    assert_eq!(active_revision, Some(fixture.revision_id));
    assert_eq!(old_state, "ready");

    let retry = evidence
        .retry
        .expect("failed B records a revision-scoped retry snapshot");
    assert!(retry.0 >= 1, "failed B increments durable retry streak");
    let failed_at = evidence
        .failed_at
        .expect("failed B includes durable failure timestamp");
    let minimum_retry_delay = match retry.0 {
        1 => 1,
        2 => 2,
        3 => 4,
        4 => 8,
        5 => 16,
        6 => 32,
        _ => 60,
    };
    let next_retry_at = retry
        .1
        .expect("failed B retry snapshot includes next retry timestamp");
    assert!(
        next_retry_at >= failed_at + time::Duration::seconds(minimum_retry_delay),
        "failed B retry must honor durable failure backoff"
    );

    let after_cleanup = exercise_gateway_service_requests(public_url).await;
    assert_eq!(after_cleanup.startup_id, first_proof.startup_id);
    let old_fence_after: i64 = sqlx::query_scalar(
        "SELECT fencing_token
           FROM gateway_service_instances
          WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
    )
    .bind(old_instance_id)
    .bind(fixture.gateway_id)
    .bind(fixture.revision_id)
    .fetch_one(pool)
    .await
    .expect("read A fencing token after failed B cleanup");
    assert_eq!(old_fence_after, old_fencing_token);

    let b_paths = gateway_service_resource_paths(evidence.instance_id);
    assert!(!b_paths.0.exists(), "failed B VM runtime is cleaned");
    assert!(!b_paths.1.exists(), "failed B cgroup is cleaned");
    assert!(!b_paths.2.exists(), "failed B materializer is cleaned");

    daemon.graceful_shutdown().await;
    wait_for_gateway_service_cleaned(pool, fixture, old_instance_id).await;
    assert!(!old_paths.0.exists(), "A VM runtime is cleaned at shutdown");
    assert!(!old_paths.1.exists(), "A cgroup is cleaned at shutdown");
    assert!(
        !old_paths.2.exists(),
        "A materializer is cleaned at shutdown"
    );
    eprintln!(
        "persistent-service-failed-candidate-passed old_instance={old_instance_id} candidate_instance={} candidate_fence={} failure_code={failure_code} old_startup_id={}",
        evidence.instance_id, evidence.fencing_token, first_proof.startup_id
    );
}
