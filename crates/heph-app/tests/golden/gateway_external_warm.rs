use super::*;

/// Runs the first persistent-service proof in a real external daemon process.
///
/// Keeping this opt-in path beside the existing golden setup is deliberate:
/// the test still owns the exact release/declaration fixture, while the child
/// consumes only the production environment contract used by `hephaestusd`.
/// Later tests can replace the graceful signal below with an unclean process
/// termination without changing the fixture or Caddy wiring.
// Keep the external daemon fixture arguments explicit so each owned resource
// and its cleanup boundary remain visible at the opt-in proof entry point.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub async fn exercise_external_gateway_service_warm_path(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    gateway: &GatewayEdgeConfig,
    app_config: &AppConfig,
    root: &Path,
    root_image: &Path,
    release_artifact_root: &Path,
    owner: &AuthenticatedIdentity,
) {
    let mut daemon =
        spawn_external_golden_daemon(gateway, app_config, root, root_image, None).await;
    wait_for_external_daemon_health(&mut daemon).await;
    let first_instance_id = wait_for_gateway_service_ready(pool, fixture).await;
    let paths = gateway_service_resource_paths(first_instance_id);
    assert!(paths.0.is_dir(), "external service VM runtime exists");
    assert!(paths.1.is_dir(), "external service cgroup exists");
    assert!(paths.2.is_dir(), "external service materializer exists");

    let applied_config =
        wait_for_caddy_configuration(&gateway.caddy_admin_url, "/gateway/service").await;
    assert!(
        applied_config.contains("/gateway/service"),
        "external daemon must publish the persistent service Caddy route"
    );
    let public_url = env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL")
        .expect("joined Caddy public URL for external daemon proof");
    let first_proof = exercise_gateway_service_requests(&public_url).await;
    let cutover = env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_CUTOVER_E2E").as_deref() == Ok("1");
    let rollback = env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_ROLLBACK_E2E").as_deref() == Ok("1");
    let unclean_restart =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_EXTERNAL_CRASH_E2E").as_deref() == Ok("1");
    let failed_candidate =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_FAILED_CANDIDATE_E2E").as_deref() == Ok("1");
    let candidate_capacity =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_CANDIDATE_CAPACITY_E2E").as_deref() == Ok("1");
    let revocation =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_REVOCATION_E2E").as_deref() == Ok("1");
    if revocation {
        service_revocation::exercise_external_gateway_service_revocation(
            pool,
            fixture,
            &daemon,
            first_instance_id,
            &paths,
            &public_url,
            owner,
        )
        .await;
        daemon.graceful_shutdown().await;
        eprintln!(
            "REAL_GATEWAY_SERVICE_REVOCATION_E2E=1 gateway_id={} revision_id={} instance_id={}",
            fixture.gateway_id, fixture.revision_id, first_instance_id
        );
    } else if failed_candidate {
        exercise_external_gateway_service_failed_candidate(
            pool,
            fixture,
            daemon,
            first_instance_id,
            paths,
            &first_proof,
            &public_url,
            release_artifact_root,
        )
        .await;
    } else if cutover {
        exercise_external_gateway_service_cutover(
            pool,
            fixture,
            gateway,
            app_config,
            root,
            root_image,
            release_artifact_root,
            daemon,
            first_instance_id,
            paths,
            &first_proof,
            &public_url,
            rollback,
            candidate_capacity,
        )
        .await;
    } else if unclean_restart {
        exercise_external_gateway_service_unclean_restart(ExternalGatewayServiceUncleanRestart {
            pool,
            fixture,
            gateway,
            app_config,
            root,
            root_image,
            daemon,
            first_instance_id,
            old_paths: paths,
            first_proof: &first_proof,
            public_url: &public_url,
        })
        .await;
    } else {
        daemon.graceful_shutdown().await;
        wait_for_gateway_service_cleaned(pool, fixture, first_instance_id).await;
        assert!(!paths.0.exists(), "external service VM runtime is cleaned");
        assert!(!paths.1.exists(), "external service cgroup is cleaned");
        assert!(
            !paths.2.exists(),
            "external service materializer is cleaned"
        );
    }
    assert!(!first_proof.startup_id.is_empty());
    eprintln!(
        "persistent-service-external-warm-evidence instance={first_instance_id} startup_id={}",
        first_proof.startup_id
    );
    eprintln!("persistent-service-external-warm-passed");
}

pub struct ExternalGatewayServiceUncleanRestart<'a> {
    pub pool: &'a sqlx::PgPool,
    pub fixture: &'a GatewayServiceGoldenFixture,
    pub gateway: &'a GatewayEdgeConfig,
    pub app_config: &'a AppConfig,
    pub root: &'a Path,
    pub root_image: &'a Path,
    pub daemon: ExternalGoldenDaemon,
    pub first_instance_id: uuid::Uuid,
    pub old_paths: (PathBuf, PathBuf, PathBuf),
    pub first_proof: &'a GatewayServiceRequestProof,
    pub public_url: &'a str,
}
