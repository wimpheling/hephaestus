use super::*;

/// Provisions the canonical cooking instance, broker, and gateway declarations.
pub struct CookingInstanceSetup {
    pub instance: cooking_builds::PreparedCookingInstance,
    pub actual_instance: SeededInstance,
    pub actual_brokered: BrokeredFixture,
    pub cooking_update_rule_ids: Option<(
        cooking_updates::BrokeredRuleIds,
        cooking_updates::BrokeredRuleIds,
        cooking_updates::BrokeredRuleIds,
        cooking_updates::BrokeredRuleIds,
    )>,
    pub cooking_inbound_placeholder: String,
    pub installed_gateway: cooking_builds::InstalledCookingGateway,
    pub foreign_instance: cooking_builds::PreparedCookingInstance,
    pub actual_grant_id: Option<uuid::Uuid>,
}

#[allow(
    clippy::needless_borrow,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub async fn prepare_cooking_instance_setup(
    pool: &sqlx::PgPool,
    cooking_context: &cooking_builds::CookingBuildContext<'_>,
    user_id: UserId,
    organization_id: OrganizationId,
    project_id: ProjectId,
    browser_e2e: bool,
    builds: &cooking_builds::PublishedCookingBuilds,
    blog_repository: &cooking_builds::PreparedCookingBlog,
    update_builds: Option<&cooking_builds::PublishedCookingUpdateBuilds>,
) -> CookingInstanceSetup {
    let instance = cooking_builds::prepare_cooking_instance(
        &cooking_context,
        builds.agent.release_agent_id,
        blog_repository.repository_id,
        cooking_builds::cooking_agent_parameters(),
    )
    .await
    .expect("ImportAgent/CreateAttachment/CreateMailbox cooking instance");
    let actual_instance = SeededInstance {
        instance: instance.instance_id,
        revision: instance.revision_id,
        attachment: instance.attachment_id,
        release: builds.agent.release_id,
        release_agent: builds.agent.release_agent_id,
    };
    let actual_brokered = cooking::seed_brokered_fixture(
        pool,
        user_id,
        organization_id,
        project_id.as_uuid(),
        &actual_instance,
    )
    .await;
    let cooking_update_rule_ids = update_builds.map(|_| {
        (
            cooking_updates::BrokeredRuleIds::fresh(),
            cooking_updates::BrokeredRuleIds::fresh(),
            cooking_updates::BrokeredRuleIds::fresh(),
            cooking_updates::BrokeredRuleIds::fresh(),
        )
    });
    if let Some((migration, rollback, abnormal, browser)) = cooking_update_rule_ids {
        actual_brokered.upstream.register_rule_copies(&[
            (cooking::MODEL_RULE, migration.model),
            (cooking::RELAY_RULE, migration.relay),
            (migration.model, rollback.model),
            (migration.relay, rollback.relay),
            (migration.model, abnormal.model),
            (migration.relay, abnormal.relay),
            (migration.model, browser.model),
            (migration.relay, browser.relay),
        ]);
    }
    let cooking_inbound_placeholder =
        cooking_builds::cooking_inbound_placeholder(actual_brokered.version_id);
    let installed_gateway = cooking_builds::install_cooking_gateway(
        &cooking_context,
        builds.gateway.release_id,
        builds.gateway.repository_id,
    )
    .await
    .expect("install released cooking gateway");
    // The browser owns the first configuration when it is enabled. This
    // keeps its immutable-revision proof independent from the ordinary
    // RPC configure/replay proof used by the backend-only path.
    let mut actual_grant_id = None;
    if browser_e2e {
        assert_eq!(
            installed_gateway.revision_id,
            sqlx::query_scalar::<_, uuid::Uuid>(
                "SELECT active_revision_id FROM gateways WHERE id = $1",
            )
            .bind(installed_gateway.gateway_id)
            .fetch_one(pool)
            .await
            .expect("installed cooking gateway active revision"),
            "browser must start from the installed, unconfigured gateway revision"
        );
        let initial_binding_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM gateway_mailbox_bindings WHERE gateway_revision_id = $1",
        )
        .bind(installed_gateway.revision_id)
        .fetch_one(pool)
        .await
        .expect("installed cooking gateway initial bindings");
        assert_eq!(
            initial_binding_count, 0,
            "browser must start from a gateway revision without mailbox bindings"
        );
    } else {
        let configured_revision = cooking_builds::configure_cooking_gateway(
            &cooking_context,
            installed_gateway,
            cooking_builds::cooking_gateway_parameters(&cooking_inbound_placeholder, 1001, 1002),
            actual_brokered.import_id,
            actual_brokered.version_id,
            instance.mailbox_id,
        )
        .await
        .expect("ConfigureGateway/CreateMailboxBinding cooking gateway");
        assert_ne!(
            configured_revision.revision_id,
            installed_gateway.revision_id
        );
        assert!(!configured_revision.grant_id.is_nil());
        actual_grant_id = Some(configured_revision.grant_id);
    }
    assert_ne!(instance.instance_id, uuid::Uuid::nil());
    // Provision a real second mailbox through the instance RPC and point
    // a temporary immutable gateway revision at it. The released source
    // then asks the host to publish through an undeclared slot; the edge
    // must reject it before creating a foreign event or run.
    let foreign_instance = cooking_builds::prepare_cooking_instance_variant(
        &cooking_context,
        builds.agent.release_agent_id,
        blog_repository.repository_id,
        cooking_builds::cooking_agent_parameters(),
        "cooking-agent-foreign",
        "cooking-agent-foreign",
    )
    .await
    .expect("provision adversarial foreign mailbox");
    assert_ne!(foreign_instance.instance_id, instance.instance_id);
    assert_ne!(foreign_instance.mailbox_id, instance.mailbox_id);
    CookingInstanceSetup {
        instance,
        actual_instance,
        actual_brokered,
        cooking_update_rule_ids,
        cooking_inbound_placeholder,
        installed_gateway,
        foreign_instance,
        actual_grant_id,
    }
}
