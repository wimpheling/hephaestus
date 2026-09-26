use super::*;

#[allow(
    clippy::cognitive_complexity,
    clippy::drop_non_drop,
    clippy::fn_params_excessive_bools,
    clippy::needless_borrow,
    clippy::question_mark,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub async fn prepare_cooking_flow(input: CookingFlowInput) -> Option<CookingPreparation> {
    let CookingFlowInput {
        pool,
        running,
        app_config,
        database_url,
        nats_url,
        root,
        fixture_repository,
        browser_oidc_issuer,
        user_id,
        organization_id,
        owner_browser_session,
        outsider_id,
        outsider_browser_session,
        project,
        observer,
        workload_phase_timing,
        release_build_proof,
        libkrun_e2e,
        session_chat_e2e: _session_chat_e2e,
        installed_ui_fixture: _installed_ui_fixture,
        browser_e2e,
        cooking_service_build_proof,
        caddy_tls,
        cooking_wait_timeout,
    } = input;
    let token = signed_token(if release_build_proof {
        Duration::from_secs(45 * 60)
    } else {
        Duration::from_secs(5 * 60)
    });
    let installed_ui_fixture =
        env::var("HEPHAESTUS_COOKING_INSTALLED_UI_FIXTURE").as_deref() == Ok("1");
    if installed_ui_fixture {
        assert!(
            browser_e2e,
            "installed UI fixture requires the browser E2E phase to be enabled"
        );
    }
    let source_root =
        PathBuf::from(env::var("HEPHAESTUS_COOKING_SOURCE_ROOT").expect("cooking source root"));
    let identity = AuthenticatedIdentity::new(
        user_id,
        &browser_oidc_issuer,
        "golden-subject",
        serde_json::json!({}),
        RequestId::new(),
    );
    let rpc_token: RpcTokenFactory = Arc::new(move |audience: &str| {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        encode(
            &Header::new(Algorithm::HS256),
            &serde_json::json!({
                "iss": "hephaestus-web-mediator",
                "sub": user_id.to_string(),
                "aud": audience,
                "iat": now,
                "nbf": now,
                "exp": now + 25,
                "jti": uuid::Uuid::new_v4().to_string(),
                "sid": owner_browser_session.to_protocol_string()
            }),
            &EncodingKey::from_secret(&hephaestus_app::rpc::mediator_signing_key(
                b"golden-internal-command-token-with-sufficient-entropy",
            )),
        )
        .expect("sign cooking build-proof mediator token")
    });
    if env::var("HEPHAESTUS_COOKING_OCI_BASE_IMPORT_DIAGNOSTIC").as_deref() == Ok("1") {
        let context = cooking_builds::CookingBuildContext {
            pool: &&pool,
            running: &running,
            root: &root,
            source_root: &source_root,
            project_id: project.id,
            repositories: &fixture_repository,
            identity: cooking_builds::CookingIdentity {
                actor: &identity,
                git_token: &token,
                rpc_token: rpc_token.as_ref(),
            },
            timeout: cooking_wait_timeout,
        };
        cooking_builds::create_cooking_blog_repository(&context)
            .await
            .expect("targeted cooking OCI base-import diagnostic");
        running
            .shutdown()
            .await
            .expect("targeted OCI diagnostic shutdown");
        cleanup_streams(&nats_url).await;
        return None;
    }
    let retry_fixture =
        if browser_e2e || env::var("HEPHAESTUS_COOKING_UPDATE_E2E").as_deref() == Ok("1") {
            let retry_repository = fixture_repository
                .create_repository_trusted(&CreateRepository {
                    project_id: project.id,
                    name: format!("golden-retry-source-{}", uuid::Uuid::new_v4()),
                    default_branch: GitRef::parse("refs/heads/main").expect("default ref"),
                    is_public: false,
                    agent_runs_enabled: true,
                })
                .await
                .expect("seed browser retry source repository");
            let retry_instance = seed_reusable_instance(
                &pool,
                user_id,
                project.id.as_uuid(),
                retry_repository.id.as_uuid(),
                &root.join("release-artifacts"),
                libkrun_e2e,
                Some(GOLDEN_AGENT.as_bytes()),
                "golden-retry-source",
            )
            .await;
            let source = create_forge_source_run(
                &pool,
                &running,
                &root,
                retry_repository.id.as_uuid(),
                retry_instance.instance,
                &token,
                libkrun_e2e,
                true,
                ForgeSourceContent::GoldenAgent,
            )
            .await;
            Some(ForgeRetryFixture {
                repository_id: retry_repository.id.as_uuid(),
                instance: retry_instance,
                source_run_id: source.run_id,
            })
        } else {
            None
        };
    let cooking_context = cooking_builds::CookingBuildContext {
        pool: &&pool,
        running: &running,
        root: &root,
        source_root: &source_root,
        project_id: project.id,
        repositories: &fixture_repository,
        identity: cooking_builds::CookingIdentity {
            actor: &identity,
            git_token: &token,
            rpc_token: rpc_token.as_ref(),
        },
        timeout: cooking_wait_timeout,
    };
    let installed_reference_uis = if installed_ui_fixture {
        Some(
            cooking_builds::build_and_install_reference_uis(&cooking_context, organization_id)
                .await
                .expect("build and install reference UIs through production boundaries"),
        )
    } else {
        None
    };
    drop(cooking_context);
    let Some(service_phase) = run_cooking_service_proof(
        &pool,
        running,
        app_config,
        cooking_service_build_proof,
        caddy_tls,
        workload_phase_timing,
        &database_url,
        &nats_url,
        &root,
        &source_root,
        project.id,
        organization_id,
        user_id,
        &fixture_repository,
        &identity,
        &token,
        rpc_token.as_ref(),
        cooking_wait_timeout,
        installed_reference_uis,
    )
    .await
    else {
        return None;
    };
    let running = service_phase.running;
    let mut app_config = service_phase.app_config;
    let installed_reference_uis = service_phase.installed_reference_uis;
    let cooking_context = cooking_builds::CookingBuildContext {
        pool: &&pool,
        running: &running,
        root: &root,
        source_root: &source_root,
        project_id: project.id,
        repositories: &fixture_repository,
        identity: cooking_builds::CookingIdentity {
            actor: &identity,
            git_token: &token,
            rpc_token: rpc_token.as_ref(),
        },
        timeout: cooking_wait_timeout,
    };
    // These repositories share only their project: Python/Rust release builds
    // do not consume the blog's Hugo image. Keep same-family release mutations
    // serial while the independent OCI build and verification make progress.
    let (
        builds,
        adversarial_agent_build,
        adversarial_gateway_build,
        update_builds,
        blog_repository,
    ) = prepare_cooking_releases(
        &pool,
        &running,
        &root,
        &source_root,
        project.id,
        user_id,
        &fixture_repository,
        &identity,
        &token,
        rpc_token.as_ref(),
        cooking_wait_timeout,
        workload_phase_timing,
    )
    .await;
    cooking_builds::wait_for_cooking_build_quiescence(
        &pool,
        project.id,
        "golden-cooking-oci-materialization",
        cooking_wait_timeout,
    )
    .await;
    let CookingInstanceSetup {
        instance,
        actual_instance,
        actual_brokered,
        cooking_update_rule_ids,
        cooking_inbound_placeholder,
        installed_gateway,
        foreign_instance,
        mut actual_grant_id,
    } = prepare_cooking_instance_setup(
        &pool,
        &cooking_context,
        user_id,
        organization_id,
        project.id,
        browser_e2e,
        &builds,
        &blog_repository,
        update_builds.as_ref(),
    )
    .await;
    actual_grant_id = run_initial_cooking_browser_phase(
        &pool,
        &database_url,
        &running,
        &root,
        browser_e2e,
        installed_reference_uis,
        organization_id,
        project.id,
        user_id,
        &builds,
        &instance,
        &actual_brokered,
        &cooking_inbound_placeholder,
        installed_gateway,
        workload_phase_timing,
        actual_grant_id,
    )
    .await;
    let actual_fixture = GatewayGoldenFixture {
        mailbox_id: MailboxId::from_uuid(instance.mailbox_id),
        grant_id: actual_grant_id.expect("cooking gateway mailbox grant after setup"),
    };
    let AdversarialSetup {
        configured: adversarial_configured,
        foreign_mailbox: adversarial_foreign_mailbox,
        canonical_revision_id,
        instance: adversarial_instance,
        rule_id: adversarial_rule_id,
    } = prepare_adversarial_setup(
        &pool,
        &cooking_context,
        &actual_brokered,
        &cooking_inbound_placeholder,
        &foreign_instance,
        &adversarial_gateway_build,
        &actual_instance,
        &adversarial_agent_build,
        &blog_repository,
        project.id,
        cooking_wait_timeout,
        &mut app_config,
    )
    .await;
    Some(CookingPreparation {
        pool,
        running: Some(running),
        app_config: Some(app_config),
        database_url,
        nats_url,
        root,
        source_root,
        fixture_repository,
        browser_oidc_issuer,
        user_id,
        organization_id,
        owner_browser_session,
        outsider_id,
        outsider_browser_session,
        project,
        observer,
        workload_phase_timing,
        browser_e2e,
        cooking_wait_timeout,
        identity,
        token,
        rpc_token,
        retry_fixture,
        builds,
        blog_repository,
        instance,
        actual_instance,
        actual_brokered,
        cooking_update_rule_ids,
        cooking_inbound_placeholder,
        actual_fixture,
        adversarial_configured,
        adversarial_foreign_mailbox,
        canonical_revision_id,
        adversarial_instance,
        adversarial_rule_id,
        update_builds,
    })
}
