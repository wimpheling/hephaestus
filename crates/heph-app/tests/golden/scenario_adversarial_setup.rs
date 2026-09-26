use super::*;

/// The adversarial gateway is configured before the daemon restart so the
/// subsequent ingress proves that an undeclared publication slot is denied.
pub struct AdversarialSetup {
    pub configured: cooking_builds::ConfiguredCookingGateway,
    pub foreign_mailbox: uuid::Uuid,
    pub canonical_revision_id: uuid::Uuid,
    pub instance: cooking_builds::PreparedCookingInstance,
    pub rule_id: uuid::Uuid,
}

#[allow(clippy::too_many_arguments)]
pub async fn prepare_adversarial_setup(
    pool: &sqlx::PgPool,
    cooking_context: &cooking_builds::CookingBuildContext<'_>,
    actual_brokered: &BrokeredFixture,
    cooking_inbound_placeholder: &str,
    foreign_instance: &cooking_builds::PreparedCookingInstance,
    adversarial_gateway_build: &cooking_builds::PublishedCookingRepository,
    actual_instance: &SeededInstance,
    adversarial_agent_build: &cooking_builds::PublishedCookingRepository,
    blog_repository: &cooking_builds::PreparedCookingBlog,
    project_id: ProjectId,
    cooking_wait_timeout: Duration,
    app_config: &mut AppConfig,
) -> AdversarialSetup {
    // Run the adversarial release only after the optional browser phase:
    // the browser is allowed to install/configure the canonical release,
    // and must not accidentally make this authority probe a no-op.
    let adversarial_installed_gateway = cooking_builds::install_cooking_gateway(
        cooking_context,
        adversarial_gateway_build.release_id,
        adversarial_gateway_build.repository_id,
    )
    .await
    .expect("install adversarial foreign-slot gateway release");
    let configured = cooking_builds::configure_cooking_gateway(
        cooking_context,
        adversarial_installed_gateway,
        cooking_builds::cooking_gateway_parameters(cooking_inbound_placeholder, 1001, 1002),
        actual_brokered.import_id,
        actual_brokered.version_id,
        foreign_instance.mailbox_id,
    )
    .await
    .expect("configure adversarial foreign publication slot");
    let installed_release: uuid::Uuid =
        sqlx::query_scalar("SELECT release_id FROM gateway_revisions WHERE id = $1")
            .bind(configured.revision_id)
            .fetch_one(pool)
            .await
            .expect("adversarial revision release provenance");
    assert_eq!(installed_release, adversarial_gateway_build.release_id);
    let has_foreign_binding: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM gateway_mailbox_bindings binding
             JOIN gateway_mailbox_binding_grants grant_row
               ON grant_row.binding_id = binding.id
             WHERE binding.gateway_revision_id = $1
               AND binding.slot_key = 'cooking_requests'
               AND binding.mailbox_id = $2
               AND grant_row.status = 'active'
         )",
    )
    .bind(configured.revision_id)
    .bind(foreign_instance.mailbox_id)
    .fetch_one(pool)
    .await
    .expect("adversarial foreign mailbox binding");
    assert!(
        has_foreign_binding,
        "foreign mailbox positive control binding"
    );
    let dispatcher = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("reserve cooking gateway dispatcher listener");
    let dispatcher_listen = dispatcher
        .local_addr()
        .expect("cooking gateway dispatcher listener address");
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
        ui_origin: None,
    });
    app_config.secret_broker_adapter = actual_brokered.upstream.adapter();
    // Finish the separate adversarial instance's immutable bindings and
    // rules before the daemon restart. The later ingress uses the restarted
    // daemon, but this setup remains tied to the published release and final
    // pre-restart revision.
    let canonical_revision_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT active_revision_id
           FROM agent_instances
          WHERE id = $1",
    )
    .bind(actual_instance.instance)
    .fetch_one(pool)
    .await
    .expect("canonical active instance revision");
    let (instance, rule_id) = cooking_adversarial_agent::prepare_adversarial_instance(
        cooking_context,
        adversarial_agent_build.release_agent_id,
        blog_repository.repository_id,
        canonical_revision_id,
        "cooking-agent-adversarial",
        "cooking-agent-adversarial",
    )
    .await
    .expect("prepare adversarial cooking agent bindings and rules");
    cooking_builds::wait_for_cooking_build_quiescence(
        pool,
        project_id,
        "golden-cooking-oci-materialization",
        cooking_wait_timeout,
    )
    .await;
    AdversarialSetup {
        configured,
        foreign_mailbox: foreign_instance.mailbox_id,
        canonical_revision_id,
        instance,
        rule_id,
    }
}
