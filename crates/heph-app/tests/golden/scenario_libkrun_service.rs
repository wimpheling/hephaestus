use super::*;

/// Runs persistent service readiness, restart, and crash lifecycle checks.
#[allow(
    clippy::cognitive_complexity,
    clippy::fn_params_excessive_bools,
    clippy::needless_borrow,
    clippy::ref_option,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub async fn run_gateway_service_lifecycle(
    pool: &sqlx::PgPool,
    mut running: hephaestus_app::RunningHephaestus,
    root: &Path,
    app_config: &AppConfig,
    gateway_caddy_e2e: bool,
    gateway_service_fixture: &Option<GatewayServiceGoldenFixture>,
    gateway_service_e2e: bool,
    gateway_service_log_guest_e2e: bool,
    gateway_service_log_rpc_e2e: bool,
    project_id: ProjectId,
    user_id: UserId,
    owner_browser_session: &BrowserSessionSid,
    outsider_id: UserId,
    outsider_browser_session: &BrowserSessionSid,
    nats_url: &str,
    mut service_instance_id: Option<uuid::Uuid>,
    mut service_resource_paths: Option<(PathBuf, PathBuf, PathBuf)>,
) -> Option<GatewayServiceLifecycleState> {
    let mut public_url = None;
    if gateway_caddy_e2e {
        let mut service_startup_id_before_crash = None;
        let admin_url =
            env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL").expect("joined Caddy admin URL");
        let applied_config = wait_for_caddy_configuration(&admin_url, "/gateway/brokered").await;
        assert!(
            applied_config.contains("/gateway/brokered"),
            "Caddy must contain the authoritative gateway route before the public proof: {applied_config}"
        );
        if gateway_service_fixture.is_some() {
            let applied_config = wait_for_caddy_configuration(&admin_url, "/gateway/service").await;
            assert!(
                applied_config.contains("/gateway/service"),
                "Caddy must contain the persistent service route before the public proof: {applied_config}"
            );
            let first_service_proof = exercise_gateway_service_requests(
                &env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("joined Caddy public URL"),
            )
            .await;
            #[cfg(feature = "test-fixtures")]
            if gateway_service_log_guest_e2e {
                golden_scenario_libkrun_service_log::run_gateway_service_guest_log_phase(
                    pool,
                    running,
                    gateway_service_fixture
                        .as_ref()
                        .expect("persistent service fixture for guest log proof"),
                    service_instance_id.expect("persistent service instance for guest log proof"),
                    service_resource_paths
                        .as_ref()
                        .expect("persistent service paths for guest log proof"),
                    project_id,
                    user_id,
                    owner_browser_session,
                    nats_url,
                )
                .await;
                return None;
            }
            #[cfg(feature = "test-fixtures")]
            let app_pool_probe = if gateway_service_log_rpc_e2e {
                let service_fixture = gateway_service_fixture
                    .as_ref()
                    .expect("persistent service fixture for log RPC");
                let (app_pool, rpc_fixture) = prepare_gateway_service_log_rpc(
                    pool,
                    &running,
                    service_fixture.gateway_id,
                    service_fixture.revision_id,
                    service_instance_id.expect("persistent service instance for log RPC"),
                    project_id.as_uuid(),
                )
                .await;
                let member_id = rpc_fixture.member_id;
                gateway_service_log_rpc::exercise_gateway_service_log_rpc(
                    &running,
                    rpc_fixture,
                    user_id.as_uuid(),
                    *owner_browser_session,
                    outsider_id.as_uuid(),
                    *outsider_browser_session,
                    || async {
                        sqlx::query(
                            "DELETE FROM project_maintainers WHERE project_id = $1 AND user_id = $2",
                        )
                        .bind(project_id.as_uuid())
                        .bind(member_id)
                        .execute(pool)
                        .await
                        .expect("revoke service log RPC member");
                    },
                )
                .await;
                Some(app_pool)
            } else {
                None
            };
            service_startup_id_before_crash = Some(first_service_proof.startup_id.clone());
            if gateway_service_e2e {
                let service_fixture = gateway_service_fixture
                    .as_ref()
                    .expect("persistent service fixture after first proof");
                let first_instance_id =
                    service_instance_id.expect("persistent service instance after first proof");
                let first_resource_paths = service_resource_paths
                    .as_ref()
                    .expect("persistent service paths after first proof");
                running
                    .shutdown()
                    .await
                    .expect("first persistent service daemon shutdown");
                #[cfg(feature = "test-fixtures")]
                if let Some(app_pool_probe) = app_pool_probe {
                    assert!(
                        app_pool_probe.is_closed(),
                        "running daemon shutdown closes its application-role pool"
                    );
                }
                let (provider_runtime, cgroup, materializer) = first_resource_paths;
                let first_state: Option<String> = sqlx::query_scalar(
                    "SELECT state FROM gateway_service_instances
                      WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
                )
                .bind(first_instance_id)
                .bind(service_fixture.gateway_id)
                .bind(service_fixture.revision_id)
                .fetch_optional(pool)
                .await
                .expect("read first cleaned persistent-service instance");
                assert_eq!(
                    first_state.as_deref(),
                    Some("cleaned"),
                    "first daemon shutdown must clean the restored service instance"
                );
                assert!(!provider_runtime.exists());
                assert!(!cgroup.exists());
                assert!(!materializer.exists());

                running = Box::pin(restart_application(app_config.clone())).await;
                let second_instance_id =
                    wait_for_gateway_service_ready(pool, service_fixture).await;
                assert_ne!(
                    first_instance_id, second_instance_id,
                    "graceful daemon restart must create a new service instance"
                );
                let second_revision_id: uuid::Uuid = sqlx::query_scalar(
                    "SELECT revision_id FROM gateway_service_instances WHERE id = $1",
                )
                .bind(second_instance_id)
                .fetch_one(pool)
                .await
                .expect("read restarted persistent-service revision");
                assert_eq!(
                    second_revision_id, service_fixture.revision_id,
                    "graceful restart must restore the same immutable service revision"
                );
                let second_resource_paths = {
                    let vm_id = format!("gateway-service-{second_instance_id}");
                    let provider_runtime_root = PathBuf::from(
                        env::var("HEPHAESTUS_LIBKRUN_RUNTIME_ROOT")
                            .expect("libkrun runtime root for restart cleanup assertion"),
                    );
                    let cgroup_root = PathBuf::from(
                        env::var("HEPHAESTUS_LIBKRUN_CGROUP_ROOT")
                            .expect("libkrun cgroup root for restart cleanup assertion"),
                    );
                    (
                        provider_runtime_root.join(&vm_id),
                        cgroup_root.join(&vm_id),
                        root.join("run-runtime")
                            .join("gateway-services")
                            .join(second_instance_id.to_string()),
                    )
                };
                let (provider_runtime, cgroup, materializer) = &second_resource_paths;
                assert!(
                    provider_runtime.is_dir(),
                    "restarted service VM runtime exists before shutdown"
                );
                assert!(
                    cgroup.is_dir(),
                    "restarted service VM cgroup exists before shutdown"
                );
                assert!(
                    materializer.is_dir(),
                    "restarted service materializer tree exists before shutdown"
                );
                let restarted_admin_url =
                    env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL").expect("joined Caddy admin URL");
                let restarted_config =
                    wait_for_caddy_configuration(&restarted_admin_url, "/gateway/service").await;
                assert!(
                    restarted_config.contains("/gateway/service"),
                    "Caddy must restore the persistent service route after daemon restart: {restarted_config}"
                );
                let second_service_proof = exercise_gateway_service_requests(
                    &env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL")
                        .expect("joined Caddy public URL after restart"),
                )
                .await;
                service_startup_id_before_crash = Some(second_service_proof.startup_id.clone());
                assert_ne!(
                    first_service_proof.startup_id, second_service_proof.startup_id,
                    "restart must replace the guest startup identity"
                );
                println!(
                    "persistent-service-restart old_instance={first_instance_id} new_instance={second_instance_id} old_startup_id={} new_startup_id={} old_pid={} new_pid={}",
                    first_service_proof.startup_id,
                    second_service_proof.startup_id,
                    first_service_proof.pid,
                    second_service_proof.pid,
                );
                service_instance_id = Some(second_instance_id);
                service_resource_paths = Some(second_resource_paths);
            }
        }
        let joined_public_url =
            env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("joined Caddy public URL");
        public_url = Some(joined_public_url);
        let public_url = public_url.as_deref().expect("joined public URL");
        if gateway_service_e2e {
            let service_fixture = gateway_service_fixture
                .as_ref()
                .expect("persistent service fixture before crash proof");
            let crashed_instance_id =
                service_instance_id.expect("persistent service instance before crash proof");
            let crashed_startup_id = service_startup_id_before_crash
                .as_deref()
                .expect("persistent service startup identity before crash");
            let crashed_resource_paths = service_resource_paths
                .as_ref()
                .expect("persistent service paths before crash proof");
            exercise_gateway_service_crash(&public_url).await;
            let replacement_instance_id = wait_for_gateway_service_crash_replacement(
                pool,
                service_fixture,
                crashed_instance_id,
            )
            .await;
            assert_ne!(
                crashed_instance_id, replacement_instance_id,
                "guest crash must create a replacement service instance"
            );
            let crash_evidence: (String, String, i32, Option<i32>) = sqlx::query_as(
                "SELECT state, failure_code, exit_code, exit_signal
                   FROM gateway_service_instances
                  WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
            )
            .bind(crashed_instance_id)
            .bind(service_fixture.gateway_id)
            .bind(service_fixture.revision_id)
            .fetch_one(pool)
            .await
            .expect("read durable guest crash evidence");
            assert_eq!(
                crash_evidence,
                (
                    String::from("cleaned"),
                    String::from("unexpected_exit"),
                    42,
                    None,
                )
            );
            let (provider_runtime, cgroup, materializer) = crashed_resource_paths;
            assert!(!provider_runtime.exists());
            assert!(!cgroup.exists());
            assert!(!materializer.exists());
            let replacement_revision_id: uuid::Uuid = sqlx::query_scalar(
                "SELECT revision_id FROM gateway_service_instances WHERE id = $1",
            )
            .bind(replacement_instance_id)
            .fetch_one(pool)
            .await
            .expect("read replacement service revision after crash");
            assert_eq!(replacement_revision_id, service_fixture.revision_id);
            let replacement_resource_paths = {
                let vm_id = format!("gateway-service-{replacement_instance_id}");
                let provider_runtime_root = PathBuf::from(
                    env::var("HEPHAESTUS_LIBKRUN_RUNTIME_ROOT")
                        .expect("libkrun runtime root for crash cleanup assertion"),
                );
                let cgroup_root = PathBuf::from(
                    env::var("HEPHAESTUS_LIBKRUN_CGROUP_ROOT")
                        .expect("libkrun cgroup root for crash cleanup assertion"),
                );
                (
                    provider_runtime_root.join(&vm_id),
                    cgroup_root.join(&vm_id),
                    root.join("run-runtime")
                        .join("gateway-services")
                        .join(replacement_instance_id.to_string()),
                )
            };
            let (provider_runtime, cgroup, materializer) = &replacement_resource_paths;
            assert!(provider_runtime.is_dir());
            assert!(cgroup.is_dir());
            assert!(materializer.is_dir());
            let replacement_proof = exercise_gateway_service_requests(&public_url).await;
            assert_ne!(
                crashed_startup_id, replacement_proof.startup_id,
                "guest crash must replace the startup identity"
            );
            println!(
                "persistent-service-crash old_instance={crashed_instance_id} replacement_instance={replacement_instance_id} exit_code=42 replacement_startup_id={} replacement_pid={}",
                replacement_proof.startup_id, replacement_proof.pid,
            );
            service_instance_id = Some(replacement_instance_id);
            service_resource_paths = Some(replacement_resource_paths);
        }
    }
    Some(GatewayServiceLifecycleState {
        running,
        service_instance_id,
        service_resource_paths,
        public_url,
    })
}

pub struct GatewayServiceLifecycleState {
    pub running: hephaestus_app::RunningHephaestus,
    pub service_instance_id: Option<uuid::Uuid>,
    pub service_resource_paths: Option<(PathBuf, PathBuf, PathBuf)>,
    pub public_url: Option<String>,
}
