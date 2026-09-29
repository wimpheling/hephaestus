use super::*;

#[allow(
    clippy::cognitive_complexity,
    clippy::needless_borrow,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub async fn run_cooking_crash_branch(prep: &mut CookingPreparation) {
    let CookingPreparation {
        pool,
        running: running_slot,
        app_config: app_config_slot,
        root,
        source_root,
        fixture_repository,
        project,
        identity,
        token,
        rpc_token,
        cooking_wait_timeout,
        actual_brokered,
        cooking_inbound_placeholder,
        instance,
        actual_instance,
        actual_fixture,
        builds,
        adversarial_instance,
        adversarial_rule_id,
        canonical_revision_id,
        blog_repository,
        owner_browser_session,
        outsider_id,
        outsider_browser_session,
        ..
    } = prep;
    let running = running_slot.take().expect("running daemon after denial");
    let pool = &*pool;
    let root = &*root;
    let source_root = &*source_root;
    let fixture_repository = &*fixture_repository;
    let project = &*project;
    let identity = &*identity;
    let token = &*token;
    let rpc_token = rpc_token.as_ref();
    let actual_brokered = &*actual_brokered;
    let cooking_inbound_placeholder = &*cooking_inbound_placeholder;
    let instance = &*instance;
    let actual_instance = &*actual_instance;
    let actual_fixture = &mut *actual_fixture;
    let builds = &*builds;
    let adversarial_instance = *adversarial_instance;
    let adversarial_rule_id = *adversarial_rule_id;
    let canonical_revision_id = *canonical_revision_id;
    let blog_repository = &*blog_repository;
    let owner_browser_session = &*owner_browser_session;
    let outsider_id = *outsider_id;
    let outsider_browser_session = *outsider_browser_session;
    let owner_browser_session = *owner_browser_session;
    let cooking_wait_timeout = *cooking_wait_timeout;
    let mut app_config = app_config_slot
        .take()
        .expect("cooking config before crash branch");
    let restored_context = cooking_builds::CookingBuildContext {
        pool,
        running: &running,
        root: &root,
        source_root: &source_root,
        project_id: project.id,
        repositories: &fixture_repository,
        identity: cooking_builds::CookingIdentity {
            actor: &identity,
            git_token: &token,
            rpc_token,
        },
        timeout: cooking_wait_timeout,
    };
    let canonical_installed_gateway = cooking_builds::install_cooking_gateway(
        &restored_context,
        builds.gateway.release_id,
        builds.gateway.repository_id,
    )
    .await
    .expect("reinstall canonical gateway after adversarial probe");
    let restored = cooking_builds::configure_cooking_gateway(
        &restored_context,
        canonical_installed_gateway,
        cooking_builds::cooking_gateway_parameters(&cooking_inbound_placeholder, 1001, 1002),
        actual_brokered.import_id,
        actual_brokered.version_id,
        instance.mailbox_id,
    )
    .await
    .expect("restore canonical gateway after adversarial probe");
    actual_fixture.grant_id = restored.grant_id;
    let checkpoint = cooking::exercise_initial(pool, &actual_fixture).await;
    cooking::wait_for_checkpoint_runs(pool, &checkpoint, restored_context.timeout).await;
    let canonical_run_ids = checkpoint.run_ids();
    // Recipe 42 above is the canonical positive control. Capture its
    // durable adapter effects before routing one mailbox-local event to a
    // separate adversarial instance. Scope both measurements to these
    // settled run IDs so unrelated canonical activity cannot move the
    // comparison window.
    let (baseline_model_calls, baseline_relay_calls): (i64, i64) = sqlx::query_as(
        "SELECT
             count(*) FILTER (WHERE audit.rule_id = $2),
             count(*) FILTER (WHERE audit.rule_id = $3)
           FROM brokered_secret_audit_events AS audit
           JOIN runs AS run ON run.id = audit.run_id
          WHERE run.id = ANY($1)
            AND audit.event_kind = 'substitution_use'",
    )
    .bind(canonical_run_ids.to_vec())
    .bind(cooking::MODEL_RULE)
    .bind(cooking::RELAY_RULE)
    .fetch_one(pool)
    .await
    .expect("canonical cooking adapter baseline");
    let (gateway_id, gateway_revision_id): (uuid::Uuid, uuid::Uuid) = sqlx::query_as(
        "SELECT gateway.id, gateway.active_revision_id
           FROM gateways AS gateway
           JOIN gateway_revisions AS revision
             ON revision.gateway_id = gateway.id
            AND revision.id = gateway.active_revision_id
           JOIN gateway_mailbox_bindings AS binding
             ON binding.gateway_revision_id = revision.id
           JOIN gateway_mailbox_binding_grants AS grant_row
             ON grant_row.binding_id = binding.id
          WHERE grant_row.id = $1
            AND grant_row.status = 'active'",
    )
    .bind(actual_fixture.grant_id)
    .fetch_one(pool)
    .await
    .expect("canonical active gateway identity");
    let adversarial_probe = cooking_adversarial_agent::exercise_adversarial_agent_probe(
        cooking_adversarial_agent::AdversarialAgentProbeInput {
            context: &restored_context,
            gateway: cooking_builds::InstalledCookingGateway {
                gateway_id,
                revision_id: gateway_revision_id,
            },
            adversarial_instance,
            canonical_run_ids,
            baseline_model_calls,
            baseline_relay_calls,
            adversarial_rule_id,
            inbound_import_id: actual_brokered.import_id,
            inbound_version_id: actual_brokered.version_id,
            inbound_placeholder: &cooking_inbound_placeholder,
            inbound_wire_credential: cooking::INBOUND_SENTINEL,
            public_url: &env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL")
                .expect("joined Caddy public URL"),
        },
    )
    .await
    .expect("adversarial cooking agent denial probe");
    assert!(!adversarial_probe.event_id.is_nil());
    assert!(!adversarial_probe.run_id.is_nil());
    assert_eq!(adversarial_probe.mismatched_rule_id, adversarial_rule_id);
    assert_eq!(adversarial_probe.deny_decisions, 1);
    assert_eq!(adversarial_probe.substitution_uses, 0);
    let restored_adversarial_gateway = cooking_builds::install_cooking_gateway(
        &restored_context,
        builds.gateway.release_id,
        builds.gateway.repository_id,
    )
    .await
    .expect("reinstall canonical gateway after agent probe");
    let restored_adversarial = cooking_builds::configure_cooking_gateway(
        &restored_context,
        restored_adversarial_gateway,
        cooking_builds::cooking_gateway_parameters(&cooking_inbound_placeholder, 1001, 1002),
        actual_brokered.import_id,
        actual_brokered.version_id,
        instance.mailbox_id,
    )
    .await
    .expect("restore canonical gateway after agent probe");
    actual_fixture.grant_id = restored_adversarial.grant_id;
    // Prepare the transformed guest and its two new rule identities before
    // the existing supervisor restart. This follows the initial and
    // adversarial controls, while canonical/crash adapters are joined by
    // immutable rule UUID and canonical counts remain unchanged.
    let crash_agent_build =
        cooking_builds::build_and_publish_guest_crash_agent(&restored_context, &builds.agent)
            .await
            .expect("publish deterministic guest crash agent");
    let crash_instance = cooking_adversarial_agent::prepare_brokered_instance_with_rule_ids(
        &restored_context,
        crash_agent_build.release_agent_id,
        blog_repository.repository_id,
        canonical_revision_id,
        "cooking-agent-guest-crash",
        "cooking-agent-guest-crash",
        cooking_adversarial_agent::BrokeredRuleIds {
            model: cooking::CRASH_MODEL_RULE,
            relay: cooking::CRASH_RELAY_RULE,
        },
    )
    .await
    .expect("prepare guest crash instance and rule specs");
    let crash_upstream = cooking::cooking_crash_upstreams(vec![
        cooking::brokered_rule_for_spec(&crash_instance.model),
        cooking::brokered_rule_for_spec(&crash_instance.relay),
    ])
    .await;
    let crash_gateway_id: uuid::Uuid =
        sqlx::query_scalar("SELECT gateway_id FROM gateway_revisions WHERE id = $1")
            .bind(restored_adversarial.revision_id)
            .fetch_one(pool)
            .await
            .expect("guest crash gateway identity");
    let crash_gateway = cooking_builds::configure_cooking_gateway(
        &restored_context,
        cooking_builds::InstalledCookingGateway {
            gateway_id: crash_gateway_id,
            revision_id: restored_adversarial.revision_id,
        },
        cooking_builds::cooking_gateway_parameters(&cooking_inbound_placeholder, 1001, 1002),
        actual_brokered.import_id,
        actual_brokered.version_id,
        crash_instance.instance.mailbox_id,
    )
    .await
    .expect("route cooking gateway to guest crash mailbox");
    app_config.secret_broker_adapter = actual_brokered.upstream.combined_adapter(&crash_upstream);
    cooking_builds::wait_for_cooking_build_quiescence(
        pool,
        project.id,
        "golden-cooking-oci-materialization",
        cooking_wait_timeout,
    )
    .await;
    running
        .shutdown()
        .await
        .expect("cooking daemon graceful restart shutdown");
    let retained_config = app_config.clone();
    let restarted = Box::pin(restart_application(app_config)).await;
    let crash_fixture = GatewayGoldenFixture {
        mailbox_id: mailbox_domain::MailboxId::from_uuid(crash_instance.instance.mailbox_id),
        grant_id: crash_gateway.grant_id,
    };
    cooking_guest_crash::exercise(pool, &crash_fixture, &crash_instance.instance).await;
    cooking_guest_crash::wait_for_upstream(crash_upstream).await;
    let crash_disk: PathBuf = PathBuf::from(
        sqlx::query_scalar::<_, String>(
            "SELECT host_path FROM agent_instance_state_volumes
          WHERE instance_id = $1",
        )
        .bind(crash_instance.instance.instance_id)
        .fetch_one(pool)
        .await
        .expect("load guest crash state volume disk path"),
    );
    let restarted_context = cooking_builds::CookingBuildContext {
        pool,
        running: &restarted,
        root: &root,
        source_root: &source_root,
        project_id: project.id,
        repositories: &fixture_repository,
        identity: cooking_builds::CookingIdentity {
            actor: &identity,
            git_token: &token,
            rpc_token,
        },
        timeout: cooking_wait_timeout,
    };
    let restored_after_crash = cooking_builds::configure_cooking_gateway(
        &restarted_context,
        cooking_builds::InstalledCookingGateway {
            gateway_id: crash_gateway_id,
            revision_id: crash_gateway.revision_id,
        },
        cooking_builds::cooking_gateway_parameters(&cooking_inbound_placeholder, 1001, 1002),
        actual_brokered.import_id,
        actual_brokered.version_id,
        instance.mailbox_id,
    )
    .await
    .expect("restore canonical gateway after guest crash branch");
    actual_fixture.grant_id = restored_after_crash.grant_id;
    let resolved_head = cooking::exercise_follow_up(
        pool,
        &restarted,
        &actual_instance,
        &actual_fixture,
        &root,
        blog_repository.repository_id.as_uuid(),
        &blog_repository.source_commit,
        checkpoint,
        &actual_brokered.upstream,
        owner_browser_session,
        outsider_id,
        outsider_browser_session,
    )
    .await;
    let blog_artifact = cooking_blog_artifact::build_publish_and_verify(
        &cooking_builds::CookingBuildContext {
            pool,
            running: &restarted,
            root: &root,
            source_root: &source_root,
            project_id: project.id,
            repositories: &fixture_repository,
            identity: cooking_builds::CookingIdentity {
                actor: &identity,
                git_token: &token,
                rpc_token,
            },
            timeout: cooking_wait_timeout,
        },
        &blog_repository,
        &resolved_head,
        "Family pasta",
        outsider_id,
        outsider_browser_session,
    )
    .await
    .expect("build and retrieve published cooking blog artifact");
    assert_eq!(blog_artifact.source_commit, resolved_head);
    assert!(!blog_artifact.build_id.is_nil());
    assert!(!blog_artifact.release_id.is_nil());
    assert!(!blog_artifact.artifact_id.is_nil());
    assert_eq!(blog_artifact.sha256.len(), 64);
    eprintln!(
        "cooking blog artifact: source_commit={}, build={}, release={}, artifact={}, sha256={}",
        blog_artifact.source_commit,
        blog_artifact.build_id,
        blog_artifact.release_id,
        blog_artifact.artifact_id,
        blog_artifact.sha256
    );

    *running_slot = Some(restarted);
    *app_config_slot = Some(retained_config);
    run_cooking_updates(
        prep,
        adversarial_probe,
        crash_agent_build,
        crash_instance,
        crash_disk,
    )
    .await;
}
