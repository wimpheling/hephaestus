use super::*;

#[allow(clippy::needless_borrow, clippy::too_many_lines)]
pub async fn run_cooking_rotation_phase(
    prep: &mut CookingPreparation,
    sequence: &cooking_updates::UpdateSequence,
) {
    let pool = &prep.pool;
    let restarted = prep.running.as_ref().expect("running daemon for rotation");
    let actual_fixture = &mut prep.actual_fixture;
    let actual_brokered = &prep.actual_brokered;
    let user_id = prep.user_id;
    let rpc_token = prep.rpc_token.as_ref();
    let instance = prep.instance;

    // The update helper rotates relay while event 47 still holds the
    // v1 model lease, before CreateUpdate clones candidate rules.
    // Event 46 remains the completed old-version relay proof.
    let relay_run_id = sequence.relay_run_id;
    let relay_rotation = sequence.relay_rotation;
    let inbound_rotation = cooking::rotate_inbound_credential(
        pool,
        user_id,
        actual_brokered.import_id,
        actual_brokered.version_id,
    )
    .await;
    *actual_fixture = cooking_authority::configure_rotated_inbound_gateway(
        pool,
        &restarted,
        &actual_fixture,
        rpc_token,
        actual_brokered.import_id,
        inbound_rotation.rotated_version_id,
        instance.mailbox_id,
    )
    .await
    .expect("configure rotated cooking inbound credential");
    let public = env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("joined Caddy public URL");
    let url = format!("{public}/gateway/cooking/telegram");
    let client = cooking::caddy_gateway_client();
    let old = cooking::send_update_with_credential(
        &client,
        &url,
        49,
        1001,
        "rotation",
        cooking::INBOUND_SENTINEL,
    )
    .await;
    assert_eq!(old.status(), reqwest::StatusCode::UNAUTHORIZED);
    old.bytes().await.expect("old inbound rotation denial");
    let accepted = cooking::send_update_with_credential(
        &client,
        &url,
        49,
        1001,
        "rotation",
        cooking::INBOUND_ROTATED_SENTINEL,
    )
    .await;
    assert_eq!(accepted.status(), reqwest::StatusCode::OK);
    accepted
        .bytes()
        .await
        .expect("rotated inbound acknowledgement");
    let rotated_run = cooking::wait_for_event_run(pool, &actual_fixture, 49).await;
    cooking::assert_rotated_brokered_lease(
        pool,
        relay_run_id,
        rotated_run.event_id,
        cooking::RELAY_RULE,
        sequence.migration.relay_rule_id,
        relay_rotation,
    )
    .await;
    cooking_authority::assert_inbound_lease_history(
        pool,
        &actual_fixture,
        inbound_rotation.pinned_version_id,
        inbound_rotation.rotated_version_id,
    )
    .await
    .expect("inbound lease rotation history");
}
