use super::*;

/// Runs the optional Cooking service proof, preserving the daemon state for the
/// ordinary Cooking build path.
#[allow(
    clippy::cognitive_complexity,
    clippy::needless_borrow,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub async fn run_cooking_service_proof(
    pool: &sqlx::PgPool,
    running: hephaestus_app::RunningHephaestus,
    mut app_config: AppConfig,
    cooking_service_build_proof: bool,
    caddy_tls: bool,
    workload_phase_timing: bool,
    database_url: &str,
    nats_url: &str,
    root: &Path,
    source_root: &Path,
    project_id: ProjectId,
    organization_id: OrganizationId,
    user_id: UserId,
    fixture_repository: &PgForgeRepository,
    identity: &AuthenticatedIdentity,
    token: &str,
    rpc_token: &(dyn Fn(&str) -> String + Send + Sync),
    cooking_wait_timeout: Duration,
    installed_reference_uis: Option<cooking_builds::InstalledCookingReferenceUis>,
) -> Option<CookingServicePhase> {
    let cooking_context = cooking_builds::CookingBuildContext {
        pool,
        running: &running,
        root,
        source_root,
        project_id,
        repositories: fixture_repository,
        identity: cooking_builds::CookingIdentity {
            actor: identity,
            git_token: token,
            rpc_token,
        },
        timeout: cooking_wait_timeout,
    };
    if cooking_service_build_proof {
        let published = cooking_service_build::build_and_publish_cooking_service(&cooking_context)
            .await
            .expect("real cooking-service source build and publish proof");
        assert_eq!(published.actor_id, user_id);
        assert!(!published.repository_id.as_uuid().is_nil());
        assert!(!published.source_commit.is_empty());
        assert!(!published.build_request_id.is_nil());
        assert!(!published.release_id.is_nil());
        assert!(!published.release_agent_id.is_nil());
        assert!(!published.version.is_empty());
        assert!(published.source_path.is_dir());
        assert!(published.working_path.is_dir());
        cooking_builds::wait_for_cooking_build_quiescence(
            pool,
            project_id,
            "golden-cooking-oci-materialization",
            cooking_wait_timeout,
        )
        .await;
        let configured = cooking_service_build::install_and_configure_cooking_service(
            &cooking_context,
            &published,
        )
        .await
        .expect("install and configure published cooking service");
        let network: String = sqlx::query_scalar(
            "SELECT release_agent.runtime_contract #>> '{policy_ceiling,network}'
                   FROM gateway_revisions revision
                   JOIN release_agents release_agent
                     ON release_agent.id = revision.release_agent_id
                  WHERE revision.id = $1",
        )
        .bind(configured.revision_id)
        .fetch_one(pool)
        .await
        .expect("read published cooking service network policy");
        assert_eq!(network, "disabled", "cooking service guest network policy");

        let dispatcher = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve cooking service dispatcher listener");
        let dispatcher_listen = dispatcher
            .local_addr()
            .expect("cooking service dispatcher listener address");
        drop(dispatcher);
        app_config.gateway_edge = Some(GatewayEdgeConfig {
            caddy_admin_url: env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL")
                .expect("joined Caddy admin URL"),
            caddy_configuration_template: caddy_configuration(
                &env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL").expect("joined Caddy admin URL"),
            ),
            caddy_server_name: String::from("shared"),
            dispatcher_listen,
            public_authority: String::from("gateway.golden.invalid"),
            ui_origin: installed_reference_uis.as_ref().map(|_| {
                let listener = reserve_installed_ui_listener();
                installed_ui_origin_config(listener)
            }),
        });
        running
            .shutdown()
            .await
            .expect("pre-Caddy cooking service daemon shutdown");
        let running = Box::pin(restart_application(app_config.clone())).await;
        let service_fixture = GatewayServiceGoldenFixture {
            gateway_id: configured.gateway_id,
            revision_id: configured.revision_id,
        };
        let service_instance_id = wait_for_gateway_service_ready(pool, &service_fixture).await;
        let pointers: (Option<uuid::Uuid>, Option<uuid::Uuid>) = sqlx::query_as(
            "SELECT active_revision_id, desired_service_revision_id
                   FROM gateways
                  WHERE id = $1",
        )
        .bind(service_fixture.gateway_id)
        .fetch_one(pool)
        .await
        .expect("read cooking service gateway revision pointers");
        assert_eq!(
            pointers,
            (
                Some(service_fixture.revision_id),
                Some(service_fixture.revision_id)
            )
        );
        let admin_url =
            env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL").expect("joined Caddy admin URL");
        let applied_config = wait_for_caddy_configuration(&admin_url, "/gateway/service").await;
        assert!(
            applied_config.contains("/gateway/service"),
            "Caddy must contain the published cooking service route: {applied_config}"
        );
        let public_url =
            env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("joined Caddy public URL");
        let client = published_cooking_service_client();
        let service_body = client
            .get(format!("{public_url}/gateway/service"))
            .send()
            .await
            .expect("public cooking service request")
            .error_for_status()
            .expect("public cooking service request succeeds")
            .bytes()
            .await
            .expect("read public cooking service response");
        assert_eq!(service_body.as_ref(), b"cooking service");
        let identity_before = exercise_published_cooking_service_identity(&public_url).await;
        exercise_published_cooking_service_isolation(&public_url, &admin_url).await;
        exercise_published_cooking_service_metadata(&public_url).await;
        // The isolation probe opens a guest-loopback request. Comparing
        // identity on both sides proves it did not replace the process.
        let identity_after = exercise_published_cooking_service_identity(&public_url).await;
        assert_eq!(identity_before.pid, identity_after.pid);
        assert_eq!(identity_before.startup_id, identity_after.startup_id);
        if caddy_tls {
            println!("REAL_COOKING_SERVICE_HTTPS_METADATA=1");
        }
        println!(
            "REAL_COOKING_SERVICE_ISOLATION=1 pid={} startup_id={}",
            identity_after.pid, identity_after.startup_id
        );
        let identity_proof = identity_after;
        if let Some(installed_uis) = installed_reference_uis {
            let managed_reference_fixture = GatewayServiceGoldenFixture {
                gateway_id: installed_uis.managed_gateway_id,
                revision_id: installed_uis.managed_gateway_revision_id,
            };
            wait_for_gateway_service_ready(pool, &managed_reference_fixture).await;
            run_installed_ui_browser_phase(InstalledUiBrowserContext {
                pool,
                running: &running,
                database_url: &database_url,
                rpc_token,
                organization_id,
                installed_uis,
                actor_id: user_id.as_uuid(),
                workload_phase_timing,
                service_materializer_root: root,
            })
            .await;
        }
        let resource_paths = {
            let vm_id = format!("gateway-service-{service_instance_id}");
            let provider_runtime_root = PathBuf::from(
                env::var("HEPHAESTUS_LIBKRUN_RUNTIME_ROOT")
                    .expect("libkrun runtime root for Cooking service proof"),
            );
            let cgroup_root = PathBuf::from(
                env::var("HEPHAESTUS_LIBKRUN_CGROUP_ROOT")
                    .expect("libkrun cgroup root for Cooking service proof"),
            );
            (
                provider_runtime_root.join(&vm_id),
                cgroup_root.join(&vm_id),
                root.join("run-runtime")
                    .join("gateway-services")
                    .join(service_instance_id.to_string()),
            )
        };
        let (provider_runtime, cgroup, materializer) = &resource_paths;
        assert!(provider_runtime.is_dir());
        assert!(cgroup.is_dir());
        assert!(materializer.is_dir());
        running
            .shutdown()
            .await
            .expect("cooking service daemon shutdown");
        let state: Option<String> = sqlx::query_scalar(
            "SELECT state FROM gateway_service_instances
                  WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
        )
        .bind(service_instance_id)
        .bind(service_fixture.gateway_id)
        .bind(service_fixture.revision_id)
        .fetch_optional(pool)
        .await
        .expect("read cleaned cooking service instance");
        assert_eq!(state.as_deref(), Some("cleaned"));
        assert!(!provider_runtime.exists());
        assert!(!cgroup.exists());
        assert!(!materializer.exists());
        cleanup_streams(nats_url).await;
        println!(
            "REAL_COOKING_SERVICE_BUILD_PROOF=1 gateway={} revision={} instance={} pid={} startup_id={}",
            service_fixture.gateway_id,
            service_fixture.revision_id,
            service_instance_id,
            identity_proof.pid,
            identity_proof.startup_id
        );
        return None;
    }
    Some(CookingServicePhase {
        running,
        app_config,
        installed_reference_uis,
    })
}

pub struct CookingServicePhase {
    pub running: hephaestus_app::RunningHephaestus,
    pub app_config: AppConfig,
    pub installed_reference_uis: Option<cooking_builds::InstalledCookingReferenceUis>,
}
