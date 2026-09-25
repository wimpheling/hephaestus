use super::*;

#[allow(
    clippy::cognitive_complexity,
    clippy::needless_borrow,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub async fn run_standard_flow(input: StandardFlowInput) {
    let StandardFlowInput {
        pool,
        running,
        app_config,
        root,
        nats_url,
        repository,
        project,
        seeded_instance,
        token,
        libkrun_e2e,
        user_id,
        owner_browser_session,
        outsider_id,
        outsider_browser_session,
        gateway_caddy_e2e,
        gateway_service_e2e,
        gateway_service_log_guest_e2e,
        gateway_service_log_rpc_e2e,
        gateway_edge,
        gateway_service_fixture,
        mut brokered_fixture,
    } = input;
    let forge_source = create_forge_source_run(
        &pool,
        &running,
        &root,
        repository.id.as_uuid(),
        seeded_instance.instance,
        &token,
        libkrun_e2e,
        false,
        if cooking::enabled() {
            ForgeSourceContent::CookingBlog
        } else {
            ForgeSourceContent::GoldenAgent
        },
    )
    .await;
    let input_commit = forge_source.input_commit;
    let run_id = runtime_types::RunId::from_uuid(forge_source.run_id);

    if env::var("HEPHAESTUS_APP_UPDATE_ADMISSION_RACE_E2E").as_deref() == Ok("1") {
        #[cfg(feature = "test-fixtures")]
        {
            let admission_instance = support::rpc::update_admission::UpdateAdmissionInstance {
                instance_id: seeded_instance.instance,
                revision_id: seeded_instance.revision,
                release_id: seeded_instance.release,
                release_agent_id: seeded_instance.release_agent,
                attachment_id: seeded_instance.attachment,
            };
            let race = support::update_admission::exercise_reconciler_wins_race(
                &pool,
                &running,
                &admission_instance,
                user_id.as_uuid(),
                owner_browser_session,
            )
            .await;
            assert_eq!(race.initial_hook_run_id, race.retried_hook_run_id);
            assert_eq!(race.retried_hook_run_id, race.owner_recovery_hook_run_id);
            running
                .shutdown()
                .await
                .expect("app update race regression shutdown");
            cleanup_streams(&nats_url).await;
            return;
        }
        #[cfg(not(feature = "test-fixtures"))]
        assert_ne!(
            env::var("HEPHAESTUS_APP_UPDATE_ADMISSION_RACE_E2E").as_deref(),
            Ok("1"),
            "HEPHAESTUS_APP_UPDATE_ADMISSION_RACE_E2E requires --features hephaestus-app/test-fixtures"
        );
    }

    if env::var("HEPHAESTUS_APP_UPDATE_ADMISSION_E2E").as_deref() == Ok("1") {
        let admission_instance = support::rpc::update_admission::UpdateAdmissionInstance {
            instance_id: seeded_instance.instance,
            revision_id: seeded_instance.revision,
            release_id: seeded_instance.release,
            release_agent_id: seeded_instance.release_agent,
            attachment_id: seeded_instance.attachment,
        };
        let admission = support::update_admission::exercise(
            &pool,
            &running,
            &admission_instance,
            user_id.as_uuid(),
            owner_browser_session,
        )
        .await;
        assert_ne!(admission.update_id, uuid::Uuid::nil());
        assert_ne!(admission.initial_hook_run_id, admission.retried_hook_run_id);
        assert_ne!(
            admission.retried_hook_run_id,
            admission.owner_recovery_hook_run_id
        );
        running
            .shutdown()
            .await
            .expect("app update regression shutdown");
        cleanup_streams(&nats_url).await;
        return;
    }

    if cooking::enabled() {
        running
            .shutdown()
            .await
            .expect("cooking daemon startup recovery shutdown");
        let running = Box::pin(restart_application(app_config.clone())).await;
        let checkpoint =
            cooking::exercise_initial(&pool, &gateway_edge.as_ref().expect("cooking gateway").1)
                .await;
        running
            .shutdown()
            .await
            .expect("cooking daemon graceful restart shutdown");
        let restarted = Box::pin(restart_application(app_config)).await;
        let _ = cooking::exercise_follow_up(
            &pool,
            &restarted,
            &seeded_instance,
            &gateway_edge.as_ref().expect("cooking gateway").1,
            &root,
            repository.id.as_uuid(),
            &input_commit,
            checkpoint,
            &brokered_fixture.as_ref().expect("cooking broker").upstream,
            owner_browser_session,
            outsider_id,
            outsider_browser_session,
        )
        .await;
        brokered_fixture
            .take()
            .expect("cooking broker")
            .upstream
            .assert_substituted_request()
            .await;
        restarted.shutdown().await.expect("cooking daemon shutdown");
        cleanup_streams(&nats_url).await;
        return;
    }

    if libkrun_e2e {
        let mut running = running;
        let (mut service_instance_id, mut service_resource_paths) = if gateway_caddy_e2e {
            let service_instance_id =
                if let Some(service_fixture) = gateway_service_fixture.as_ref() {
                    Some(wait_for_gateway_service_ready(&pool, service_fixture).await)
                } else {
                    None
                };
            let service_resource_paths = service_instance_id.map(|instance_id| {
                let vm_id = format!("gateway-service-{instance_id}");
                let provider_runtime_root = PathBuf::from(
                    env::var("HEPHAESTUS_LIBKRUN_RUNTIME_ROOT")
                        .expect("libkrun runtime root for cleanup assertion"),
                );
                let cgroup_root = PathBuf::from(
                    env::var("HEPHAESTUS_LIBKRUN_CGROUP_ROOT")
                        .expect("libkrun cgroup root for cleanup assertion"),
                );
                (
                    provider_runtime_root.join(&vm_id),
                    cgroup_root.join(&vm_id),
                    root.join("run-runtime")
                        .join("gateway-services")
                        .join(instance_id.to_string()),
                )
            });
            if let Some((provider_runtime, cgroup, materializer)) = &service_resource_paths {
                assert!(
                    provider_runtime.is_dir(),
                    "service VM runtime exists before shutdown"
                );
                assert!(cgroup.is_dir(), "service VM cgroup exists before shutdown");
                assert!(
                    materializer.is_dir(),
                    "service materializer tree exists before shutdown"
                );
            }
            (service_instance_id, service_resource_paths)
        } else {
            (None, None)
        };
        let public_url = if gateway_caddy_e2e {
            let Some(state) = run_gateway_service_lifecycle(
                &pool,
                running,
                &root,
                &app_config,
                gateway_caddy_e2e,
                &gateway_service_fixture,
                gateway_service_e2e,
                gateway_service_log_guest_e2e,
                gateway_service_log_rpc_e2e,
                project.id,
                user_id,
                &owner_browser_session,
                outsider_id,
                &outsider_browser_session,
                &nats_url,
                service_instance_id,
                service_resource_paths,
            )
            .await
            else {
                return;
            };
            running = state.running;
            service_instance_id = state.service_instance_id;
            service_resource_paths = state.service_resource_paths;
            state.public_url
        } else {
            None
        };
        run_libkrun_gateway_requests(
            &pool,
            &nats_url,
            gateway_caddy_e2e,
            &gateway_edge,
            &gateway_service_fixture,
            public_url,
            gateway_service_e2e,
            user_id,
            running,
            &mut brokered_fixture,
            service_instance_id,
            service_resource_paths,
        )
        .await;
        return;
    }

    run_result_and_mailbox_phase(
        &pool,
        &root,
        repository.id.as_uuid(),
        run_id,
        &input_commit,
        user_id,
        project.id.as_uuid(),
        &seeded_instance,
        running,
        &mut brokered_fixture,
        &nats_url,
    )
    .await;
}
