use super::*;

/// Re-selects the original immutable release after B has served. A finite B
/// hold keeps the old candidate draining long enough to prove the replacement
/// becomes Ready and active before B is retired.
// Keep the complete operator rollback evidence at one call site so its
// ordering and resource assertions remain reviewable together.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub async fn exercise_external_gateway_service_rollback(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    gateway: &GatewayEdgeConfig,
    daemon: ExternalGoldenDaemon,
    original_a_instance_id: uuid::Uuid,
    b_fixture: &GatewayServiceGoldenFixture,
    b_instance_id: uuid::Uuid,
    b_paths: (PathBuf, PathBuf, PathBuf),
    first_a_proof: &GatewayServiceRequestProof,
    b_proof: &GatewayServiceRequestProof,
    public_url: &str,
) {
    let baseline_invocations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_invocations WHERE gateway_id = $1")
            .bind(fixture.gateway_id)
            .fetch_one(pool)
            .await
            .expect("count service invocations before rollback hold");
    let hold_started_at: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(pool)
        .await
        .expect("read rollback hold start time");
    let b_fencing_token: i64 = sqlx::query_scalar(
        "SELECT fencing_token
           FROM gateway_service_instances
          WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
    )
    .bind(b_instance_id)
    .bind(b_fixture.gateway_id)
    .bind(b_fixture.revision_id)
    .fetch_one(pool)
    .await
    .expect("read B fencing token before rollback hold");
    let hold_nonce = uuid::Uuid::new_v4();
    let hold_url = format!("{public_url}/gateway/service/hold?rollback_nonce={hold_nonce}");
    let hold_deadline = tokio::time::Instant::now() + Duration::from_secs(29);
    let hold_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(29))
        .build()
        .expect("bounded rollback hold client");
    let hold = tokio::spawn(async move {
        let bytes = hold_client
            .get(hold_url)
            .send()
            .await
            .expect("public rollback hold request")
            .error_for_status()
            .expect("rollback hold succeeds")
            .bytes()
            .await
            .expect("read rollback hold response");
        let body: serde_json::Value =
            serde_json::from_slice(&bytes).expect("rollback hold identity JSON");
        GatewayServiceRequestProof {
            pid: body
                .get("pid")
                .and_then(serde_json::Value::as_u64)
                .expect("rollback hold guest PID"),
            startup_id: body
                .get("startup_id")
                .and_then(serde_json::Value::as_str)
                .expect("rollback hold guest startup identity")
                .to_owned(),
        }
    });
    let hold_invocation = wait_for_accepted_gateway_service_hold(
        pool,
        b_fixture,
        b_instance_id,
        b_fencing_token,
        hold_started_at,
        baseline_invocations,
    )
    .await;
    assert!(
        !hold.is_finished(),
        "B hold remains in flight after acceptance"
    );

    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway_id)
        .bind(fixture.revision_id)
        .execute(pool)
        .await
        .expect("declare rollback to original A revision");
    let rollback_a_instance_id = wait_for_gateway_service_cutover_state(
        pool,
        fixture.gateway_id,
        b_instance_id,
        b_fixture.revision_id,
        fixture.revision_id,
    )
    .await;
    assert_ne!(rollback_a_instance_id, original_a_instance_id);
    assert_ne!(rollback_a_instance_id, b_instance_id);
    assert!(
        !hold.is_finished(),
        "B hold remains pending during rollback"
    );
    let rollback_a_paths = gateway_service_resource_paths(rollback_a_instance_id);
    assert!(
        b_paths.0.is_dir(),
        "B VM runtime remains during rollback drain"
    );
    assert!(b_paths.1.is_dir(), "B cgroup remains during rollback drain");
    assert!(
        b_paths.2.is_dir(),
        "B materializer remains during rollback drain"
    );
    assert!(
        rollback_a_paths.0.is_dir(),
        "rollback A VM runtime is serving"
    );
    assert!(rollback_a_paths.1.is_dir(), "rollback A cgroup is serving");
    assert!(
        rollback_a_paths.2.is_dir(),
        "rollback A materializer is serving"
    );
    let a_before: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(pool)
        .await
        .expect("read rollback A public request start time");
    let rollback_a_proof = exercise_gateway_service_cutover_requests(
        pool,
        fixture,
        rollback_a_instance_id,
        public_url,
        a_before,
    )
    .await;
    assert_ne!(rollback_a_proof.startup_id, first_a_proof.startup_id);
    assert_ne!(rollback_a_proof.startup_id, b_proof.startup_id);
    assert!(
        !hold.is_finished(),
        "B hold remains pending after rollback A traffic"
    );
    let b_state: String = sqlx::query_scalar(
        "SELECT state FROM gateway_service_instances
           WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
    )
    .bind(b_instance_id)
    .bind(b_fixture.gateway_id)
    .bind(b_fixture.revision_id)
    .fetch_one(pool)
    .await
    .expect("read B state after rollback A traffic");
    assert_eq!(b_state, "draining");
    let hold_outcome: String =
        sqlx::query_scalar("SELECT outcome FROM gateway_invocations WHERE id = $1")
            .bind(hold_invocation)
            .fetch_one(pool)
            .await
            .expect("read B rollback hold outcome");
    assert_eq!(hold_outcome, "accepted");

    let hold_proof = tokio::time::timeout_at(hold_deadline, hold)
        .await
        .expect("B hold completes inside the public exchange deadline")
        .expect("B hold task joins");
    assert_eq!(hold_proof.startup_id, b_proof.startup_id);
    wait_for_gateway_invocation_completed(pool, hold_invocation).await;
    wait_for_gateway_service_cleaned(pool, b_fixture, b_instance_id).await;
    assert!(!b_paths.0.exists());
    assert!(!b_paths.1.exists());
    assert!(!b_paths.2.exists());
    assert!(rollback_a_paths.0.is_dir() && rollback_a_paths.1.is_dir());
    assert!(rollback_a_paths.2.is_dir());

    daemon.graceful_shutdown().await;
    wait_for_gateway_service_cleaned(pool, fixture, rollback_a_instance_id).await;
    assert!(!rollback_a_paths.0.exists());
    assert!(!rollback_a_paths.1.exists());
    assert!(!rollback_a_paths.2.exists());
    let applied_config =
        wait_for_caddy_configuration(&gateway.caddy_admin_url, "/gateway/service").await;
    assert!(applied_config.contains("/gateway/service"));
    eprintln!(
        "persistent-service-rollback-passed original_a_instance={original_a_instance_id} b_instance={b_instance_id} rollback_a_instance={rollback_a_instance_id} original_a_startup_id={} b_startup_id={} rollback_a_startup_id={}",
        first_a_proof.startup_id, b_proof.startup_id, rollback_a_proof.startup_id
    );
}
