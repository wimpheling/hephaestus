use super::*;

/// Extends the real A/B cutover proof with a third revision admission check.
/// A's accepted request remains unresolved while the test observes C blocked
/// by durable capacity, then admits C only after A is physically cleaned.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub async fn exercise_external_gateway_service_candidate_capacity(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    b_fixture: &GatewayServiceGoldenFixture,
    release_artifact_root: &Path,
    daemon: ExternalGoldenDaemon,
    old_instance_id: uuid::Uuid,
    old_fencing_token: i64,
    old_paths: (PathBuf, PathBuf, PathBuf),
    first_proof: &GatewayServiceRequestProof,
    public_url: &str,
    b_instance_id: uuid::Uuid,
    b_paths: (PathBuf, PathBuf, PathBuf),
    b_proof: &GatewayServiceRequestProof,
    hold: tokio::task::JoinHandle<GatewayServiceRequestProof>,
    hold_invocation: uuid::Uuid,
    hold_deadline: tokio::time::Instant,
) {
    let third =
        seed_gateway_service_cutover_candidate(pool, fixture, release_artifact_root, "/readyz")
            .await;
    let accepted_invocation: (uuid::Uuid, uuid::Uuid, uuid::Uuid, i64, String) = sqlx::query_as(
        "SELECT gateway_id, gateway_revision_id, service_instance_id,
                service_instance_fencing_token, outcome
           FROM gateway_invocations
          WHERE id = $1",
    )
    .bind(hold_invocation)
    .fetch_one(pool)
    .await
    .expect("read accepted A capacity hold");
    assert_eq!(accepted_invocation.0, fixture.gateway_id);
    assert_eq!(accepted_invocation.1, fixture.revision_id);
    assert_eq!(accepted_invocation.2, old_instance_id);
    assert_eq!(accepted_invocation.3, old_fencing_token);
    assert_eq!(accepted_invocation.4, "accepted");
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway_id)
        .bind(third.revision_id)
        .execute(pool)
        .await
        .expect("declare third candidate while A drains and B serves");

    let mut observed_blocked = false;
    let mut serving_fencing_token = None;
    tokio::time::timeout_at(hold_deadline, async {
        loop {
            let snapshot = read_external_candidate_capacity_snapshot(
                pool,
                fixture.gateway_id,
                old_instance_id,
                fixture.revision_id,
                b_instance_id,
                b_fixture.revision_id,
                third.revision_id,
            )
            .await;
            assert!(
                !(snapshot.next_instances > 0 && snapshot.old_state != "cleaned"),
                "C was admitted before A cleaned: a_state={} c_instances={}",
                snapshot.old_state,
                snapshot.next_instances
            );
            assert_eq!(snapshot.old_fencing_token, old_fencing_token);
            if let Some(expected) = serving_fencing_token {
                assert_eq!(snapshot.serving_fencing_token, expected);
            } else {
                assert!(snapshot.serving_fencing_token > 0);
                serving_fencing_token = Some(snapshot.serving_fencing_token);
            }
            let hold_unresolved = !hold.is_finished();
            if hold_unresolved
                && snapshot.active_revision == Some(b_fixture.revision_id)
                && snapshot.old_state == "draining"
                && snapshot.serving_state == "ready"
                && snapshot.next_instances == 0
            {
                observed_blocked = true;
            }
            if hold.is_finished() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("A capacity hold completes inside the public exchange deadline");

    let hold_proof = hold.await.expect("A capacity hold task joins");
    assert_eq!(hold_proof.startup_id, first_proof.startup_id);

    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let snapshot = read_external_candidate_capacity_snapshot(
                pool,
                fixture.gateway_id,
                old_instance_id,
                fixture.revision_id,
                b_instance_id,
                b_fixture.revision_id,
                third.revision_id,
            )
            .await;
            assert!(
                !(snapshot.next_instances > 0 && snapshot.old_state != "cleaned"),
                "C was admitted before A cleaned: a_state={} c_instances={}",
                snapshot.old_state,
                snapshot.next_instances
            );
            assert_eq!(snapshot.old_fencing_token, old_fencing_token);
            assert_eq!(serving_fencing_token, Some(snapshot.serving_fencing_token));
            if snapshot.old_state == "cleaned" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("A capacity hold cleanup reaches durable Cleaned state");
    assert!(
        observed_blocked,
        "at least one durable A-draining/B-ready/C-empty snapshot is required"
    );
    wait_for_gateway_invocation_completed(pool, hold_invocation).await;
    wait_for_gateway_service_cleaned(pool, fixture, old_instance_id).await;
    assert!(!old_paths.0.exists());
    assert!(!old_paths.1.exists());
    assert!(!old_paths.2.exists());

    let third_instance_id = wait_for_gateway_service_ready(pool, &third).await;
    let third_paths = gateway_service_resource_paths(third_instance_id);
    assert!(third_paths.0.is_dir());
    assert!(third_paths.1.is_dir());
    assert!(third_paths.2.is_dir());
    let third_started_at: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(pool)
        .await
        .expect("read C request start time");
    let third_proof = exercise_gateway_service_cutover_requests(
        pool,
        &third,
        third_instance_id,
        public_url,
        third_started_at,
    )
    .await;
    assert_ne!(third_proof.startup_id, first_proof.startup_id);
    assert_ne!(third_proof.startup_id, b_proof.startup_id);

    // C promotion commits the active pointer before the paced reconciliation
    // pass asks the now-noncurrent B job to drain. B may therefore remain
    // Ready at this instant; the durable cleanup and physical-path checks
    // below prove that reconciliation eventually retires it.
    wait_for_gateway_service_cleaned(pool, b_fixture, b_instance_id).await;
    assert!(!b_paths.0.exists());
    assert!(!b_paths.1.exists());
    assert!(!b_paths.2.exists());

    daemon.graceful_shutdown().await;
    wait_for_gateway_service_cleaned(pool, &third, third_instance_id).await;
    assert!(!third_paths.0.exists());
    assert!(!third_paths.1.exists());
    assert!(!third_paths.2.exists());
    eprintln!(
        "REAL_GATEWAY_SERVICE_CANDIDATE_CAPACITY_E2E=1 a_instance={old_instance_id} b_instance={b_instance_id} c_instance={third_instance_id} a_startup_id={} b_startup_id={} c_startup_id={}",
        first_proof.startup_id, b_proof.startup_id, third_proof.startup_id
    );
}
