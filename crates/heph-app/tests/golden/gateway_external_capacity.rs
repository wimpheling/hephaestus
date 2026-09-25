use super::*;

pub async fn read_gateway_service_fencing_token(
    pool: &sqlx::PgPool,
    instance_id: uuid::Uuid,
    gateway_id: uuid::Uuid,
    revision_id: uuid::Uuid,
) -> i64 {
    sqlx::query_scalar(
        "SELECT fencing_token
           FROM gateway_service_instances
          WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
    )
    .bind(instance_id)
    .bind(gateway_id)
    .bind(revision_id)
    .fetch_one(pool)
    .await
    .expect("read A fencing token during failed B cleanup")
}

/// Exercises one real public revision cutover while an accepted request is
/// still executing against the old guest.  The service fixture's bounded hold
/// response keeps the exchange buffered until the new revision is serving.
// This opt-in harness function keeps the complete external cutover evidence
// together so its cleanup ordering remains reviewable at one call site.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub async fn exercise_external_gateway_service_cutover(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    gateway: &GatewayEdgeConfig,
    _app_config: &AppConfig,
    _root: &Path,
    _root_image: &Path,
    release_artifact_root: &Path,
    daemon: ExternalGoldenDaemon,
    old_instance_id: uuid::Uuid,
    old_paths: (PathBuf, PathBuf, PathBuf),
    first_proof: &GatewayServiceRequestProof,
    public_url: &str,
    rollback: bool,
    candidate_capacity: bool,
) {
    let candidate =
        seed_gateway_service_cutover_candidate(pool, fixture, release_artifact_root, "/readyz")
            .await;
    let baseline_invocations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_invocations WHERE gateway_id = $1")
            .bind(fixture.gateway_id)
            .fetch_one(pool)
            .await
            .expect("count service invocations before cutover hold");
    let hold_started_at: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(pool)
        .await
        .expect("read cutover hold start time");
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
    .expect("read A fencing token before hold");
    let hold_nonce = uuid::Uuid::new_v4();
    let hold_url = format!("{public_url}/gateway/service/hold?nonce={hold_nonce}");
    let hold_deadline = tokio::time::Instant::now() + Duration::from_secs(29);
    let hold_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(29))
        .build()
        .expect("bounded cutover hold client");
    let hold = tokio::spawn(async move {
        let bytes = hold_client
            .get(hold_url)
            .send()
            .await
            .expect("public persistent-service hold request")
            .error_for_status()
            .expect("persistent-service hold succeeds")
            .bytes()
            .await
            .expect("read persistent-service hold response");
        let body: serde_json::Value =
            serde_json::from_slice(&bytes).expect("persistent-service hold identity JSON");
        GatewayServiceRequestProof {
            pid: body
                .get("pid")
                .and_then(serde_json::Value::as_u64)
                .expect("hold guest PID"),
            startup_id: body
                .get("startup_id")
                .and_then(serde_json::Value::as_str)
                .expect("hold guest startup identity")
                .to_owned(),
        }
    });
    let hold_invocation = wait_for_accepted_gateway_service_hold(
        pool,
        fixture,
        old_instance_id,
        old_fencing_token,
        hold_started_at,
        baseline_invocations,
    )
    .await;
    assert!(
        !hold.is_finished(),
        "A hold remains in flight after acceptance"
    );

    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway_id)
        .bind(candidate.revision_id)
        .execute(pool)
        .await
        .expect("declare published cutover candidate");
    let candidate_instance_id = wait_for_gateway_service_cutover_state(
        pool,
        fixture.gateway_id,
        old_instance_id,
        fixture.revision_id,
        candidate.revision_id,
    )
    .await;
    assert!(
        !hold.is_finished(),
        "A hold remains pending while B is ready, active, and A is draining"
    );
    let candidate_paths = gateway_service_resource_paths(candidate_instance_id);
    assert!(old_paths.0.is_dir(), "A VM runtime remains during drain");
    assert!(old_paths.1.is_dir(), "A cgroup remains during drain");
    assert!(old_paths.2.is_dir(), "A materializer remains during drain");
    assert!(
        candidate_paths.0.is_dir(),
        "B VM runtime exists while serving"
    );
    assert!(candidate_paths.1.is_dir(), "B cgroup exists while serving");
    assert!(
        candidate_paths.2.is_dir(),
        "B materializer exists while serving"
    );
    let b_before: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(pool)
        .await
        .expect("read B public request start time");
    let b_proof = exercise_gateway_service_cutover_requests(
        pool,
        &candidate,
        candidate_instance_id,
        public_url,
        b_before,
    )
    .await;
    assert_ne!(b_proof.startup_id, first_proof.startup_id);
    assert!(
        !hold.is_finished(),
        "A hold remains pending after B traffic"
    );
    let old_state: String =
        sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
            .bind(old_instance_id)
            .fetch_one(pool)
            .await
            .expect("read A state after B traffic");
    assert_eq!(old_state, "draining");
    let hold_outcome: String =
        sqlx::query_scalar("SELECT outcome FROM gateway_invocations WHERE id = $1")
            .bind(hold_invocation)
            .fetch_one(pool)
            .await
            .expect("read A hold outcome after B traffic");
    assert_eq!(hold_outcome, "accepted");
    assert!(old_paths.0.is_dir() && old_paths.1.is_dir() && old_paths.2.is_dir());

    if candidate_capacity {
        exercise_external_gateway_service_candidate_capacity(
            pool,
            fixture,
            &candidate,
            release_artifact_root,
            daemon,
            old_instance_id,
            old_fencing_token,
            old_paths,
            first_proof,
            public_url,
            candidate_instance_id,
            candidate_paths,
            &b_proof,
            hold,
            hold_invocation,
            hold_deadline,
        )
        .await;
        return;
    }

    let hold_proof = tokio::time::timeout_at(hold_deadline, hold)
        .await
        .expect("A hold completes inside the public exchange deadline")
        .expect("A hold task joins");
    assert_eq!(hold_proof.startup_id, first_proof.startup_id);
    wait_for_gateway_invocation_completed(pool, hold_invocation).await;
    wait_for_gateway_service_cleaned(pool, fixture, old_instance_id).await;
    assert!(
        !old_paths.0.exists(),
        "A VM runtime is cleaned after response"
    );
    assert!(!old_paths.1.exists(), "A cgroup is cleaned after response");
    assert!(
        !old_paths.2.exists(),
        "A materializer is cleaned after response"
    );
    assert!(
        candidate_paths.0.is_dir() && candidate_paths.1.is_dir() && candidate_paths.2.is_dir(),
        "B remains serving after A cleanup"
    );
    if rollback {
        exercise_external_gateway_service_rollback(
            pool,
            fixture,
            gateway,
            daemon,
            old_instance_id,
            &candidate,
            candidate_instance_id,
            candidate_paths,
            first_proof,
            &b_proof,
            public_url,
        )
        .await;
    } else {
        daemon.graceful_shutdown().await;
        wait_for_gateway_service_cleaned(pool, &candidate, candidate_instance_id).await;
        assert!(!candidate_paths.0.exists());
        assert!(!candidate_paths.1.exists());
        assert!(!candidate_paths.2.exists());
        let applied_config =
            wait_for_caddy_configuration(&gateway.caddy_admin_url, "/gateway/service").await;
        assert!(applied_config.contains("/gateway/service"));
        eprintln!(
            "persistent-service-cutover-passed old_instance={old_instance_id} new_instance={candidate_instance_id} old_startup_id={} new_startup_id={}",
            first_proof.startup_id, b_proof.startup_id
        );
    }
}

pub struct ExternalCandidateCapacitySnapshot {
    pub active_revision: Option<uuid::Uuid>,
    pub old_state: String,
    pub old_fencing_token: i64,
    pub serving_state: String,
    pub serving_fencing_token: i64,
    pub next_instances: i64,
}

pub async fn read_external_candidate_capacity_snapshot(
    pool: &sqlx::PgPool,
    gateway_id: uuid::Uuid,
    old_instance_id: uuid::Uuid,
    old_revision_id: uuid::Uuid,
    serving_instance_id: uuid::Uuid,
    serving_revision_id: uuid::Uuid,
    next_revision_id: uuid::Uuid,
) -> ExternalCandidateCapacitySnapshot {
    let row: (Option<uuid::Uuid>, String, i64, String, i64, i64) = sqlx::query_as(
        "SELECT gateway.active_revision_id, old_instance.state,
                old_instance.fencing_token, serving_instance.state,
                serving_instance.fencing_token, count(next_instance.id)::bigint
           FROM gateways AS gateway
           JOIN gateway_service_instances AS old_instance
             ON old_instance.id = $2
            AND old_instance.gateway_id = gateway.id
            AND old_instance.revision_id = $3
           JOIN gateway_service_instances AS serving_instance
             ON serving_instance.id = $4
            AND serving_instance.gateway_id = gateway.id
            AND serving_instance.revision_id = $5
           LEFT JOIN gateway_service_instances AS next_instance
             ON next_instance.gateway_id = gateway.id
            AND next_instance.revision_id = $6
          WHERE gateway.id = $1
          GROUP BY gateway.active_revision_id, old_instance.state,
                   old_instance.fencing_token, serving_instance.state,
                   serving_instance.fencing_token",
    )
    .bind(gateway_id)
    .bind(old_instance_id)
    .bind(old_revision_id)
    .bind(serving_instance_id)
    .bind(serving_revision_id)
    .bind(next_revision_id)
    .fetch_one(pool)
    .await
    .expect("read real candidate capacity state");
    ExternalCandidateCapacitySnapshot {
        active_revision: row.0,
        old_state: row.1,
        old_fencing_token: row.2,
        serving_state: row.3,
        serving_fencing_token: row.4,
        next_instances: row.5,
    }
}
