use super::*;

#[allow(
    clippy::needless_borrow,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub async fn run_cooking_cleanup_phase(
    prep: &mut CookingPreparation,
    update_sequence: Option<cooking_updates::UpdateSequence>,
    update_state_volume_disk: Option<PathBuf>,
    adversarial_probe: cooking_adversarial_agent::AdversarialAgentProbe,
    crash_agent_build: cooking_builds::PublishedCookingRepository,
    crash_instance: cooking_adversarial_agent::PreparedBrokeredInstance,
    crash_disk: PathBuf,
) {
    let pool = &prep.pool;
    let restarted = prep.running.take().expect("running daemon for cleanup");
    let actual_fixture = &prep.actual_fixture;
    let actual_brokered = &prep.actual_brokered;
    let rpc_token = prep.rpc_token.as_ref();
    let identity = &prep.identity;
    let user_id = prep.user_id;
    let outsider_id = prep.outsider_id;
    let actual_instance = &prep.actual_instance;
    let instance = prep.instance;
    let project = &prep.project;
    let browser_oidc_issuer = &prep.browser_oidc_issuer;
    let retry_fixture = prep.retry_fixture.as_ref();
    let nats_url = &prep.nats_url;
    let builds = &prep.builds;
    let observer = prep.observer.take();
    let inbound_credential = if update_sequence.is_some() {
        cooking::INBOUND_ROTATED_SENTINEL
    } else {
        cooking::INBOUND_SENTINEL
    };

    let retained_run_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT id
               FROM runs
              WHERE instance_id = $1
                AND run_kind = 'normal'
                AND state = 'cleaned_up'
                AND outcome = 'succeeded'
              ORDER BY updated_at DESC, id DESC
              LIMIT 1",
    )
    .bind(actual_instance.instance)
    .fetch_one(pool)
    .await
    .expect("completed cooking run retained for retirement proof");
    let gateway_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT gateway_id
               FROM gateway_mailbox_bindings
              WHERE id = (
                  SELECT binding_id
                    FROM gateway_mailbox_binding_grants
                   WHERE id = $1
              )",
    )
    .bind(actual_fixture.grant_id)
    .fetch_one(pool)
    .await
    .expect("cooking gateway identity for retirement proof");
    let outsider_identity = AuthenticatedIdentity::new(
        outsider_id,
        (*browser_oidc_issuer).clone(),
        "cooking-outsider",
        serde_json::json!({}),
        RequestId::new(),
    );
    let retirement_public = env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL")
        .expect("joined Caddy public URL for retirement proof");
    let retry_fixture = retry_fixture
        .as_ref()
        .expect("retirement retry forge fixture");
    cooking_retirement::exercise(&cooking_retirement::RetirementContext {
        pool,
        running: &restarted,
        token_factory: rpc_token,
        owner: &identity,
        outsider: &outsider_identity,
        instance: &actual_instance,
        retained_run_id,
        retry_instance: &retry_fixture.instance,
        retry_source_run_id: retry_fixture.source_run_id,
        retry_repository_id: retry_fixture.repository_id,
        gateway_id,
        project_id: project.id.as_uuid(),
        mailbox_id: instance.mailbox_id,
        public_url: &retirement_public,
        valid_inbound_credential: inbound_credential,
        import_parameters: cooking_builds::cooking_agent_parameters(),
    })
    .await
    .expect("retire cooking attachment, gateway, and release");
    // Source revocation follows grant retirement so the valid rotated
    // inbound credential proves the grant fence itself.
    cooking::revoke_imported_credential(pool, user_id, actual_brokered.import_id).await;
    cooking_confinement::assert_database_has_no_credentials(pool).await;
    restarted.shutdown().await.expect("cooking daemon shutdown");
    let observer = observer.expect("cooking build proof VM observer");
    support::vm_observer_assertions::assert_cooking_vm_contracts(
        pool,
        &observer,
        support::vm_observer_assertions::CookingVmContractIds {
            canonical_mailbox_id: instance.mailbox_id,
            crash_mailbox_id: crash_instance.instance.mailbox_id,
            adversarial_run_id: adversarial_probe.run_id,
            gateway_build_request_id: builds.gateway.build_request_id,
            agent_build_request_id: builds.agent.build_request_id,
            crash_agent_build_request_id: crash_agent_build.build_request_id,
        },
    )
    .await;
    cooking_guest_crash::assert_sqlite_disk(&crash_disk);
    cooking_confinement::assert_nats_has_no_credentials(&nats_url).await;
    if let Some(disk) = update_state_volume_disk {
        cooking_updates::assert_migrated_sqlite_disk(&disk, 9);
    }
    cleanup_streams(&nats_url).await;
}
