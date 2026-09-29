// Reuse the update facade imports across focused lifecycle phases.
#[allow(unused_imports)]
use super::*;
/// Admits an update only while a known v1 run is still active.  This is the
/// reusable production barrier: the SQL lifecycle check proves the run uses
/// the expected revision, then `CreateUpdate` must observe it during drain and
/// leave the gate closed until that run is cleaned up.
pub(crate) async fn begin_update_while_v1_active(
    context: &CookingUpdateContext<'_>,
    candidate_release_agent_id: Uuid,
    active_run_id: Uuid,
    candidate_rule_ids: BrokeredRuleIds,
) -> UpdateIds {
    assert_active_v1_run(context.pool, active_run_id, context.current_revision_id).await;
    let ids = create_update(context, candidate_release_agent_id, candidate_rule_ids).await;
    let state = load_state(context.pool, ids.update_id).await;
    assert!(!state.run_gate_open, "active v1 drain closes the run gate");
    assert_eq!(state.active_revision_id, Some(context.current_revision_id));
    assert_eq!(state.update, "draining");
    let run_state: String = sqlx::query_scalar("SELECT state FROM runs WHERE id = $1")
        .bind(active_run_id)
        .fetch_one(context.pool)
        .await
        .expect("active v1 run after update admission");
    assert!(
        [
            "queued",
            "leasing_volume",
            "provisioning",
            "starting",
            "running"
        ]
        .contains(&run_state.as_str()),
        "update must wait for the active v1 run to drain: {run_state}"
    );
    ids
}

/// Runs migration, deferred mailbox selection, explicit rollback, and
/// abnormal-hook recovery using three candidates from one published family.
/// The callback must submit a real gateway event and return its mailbox event
/// ID while the migration gate is closed; it must not wait for that run.
pub(crate) async fn exercise_update_sequence<F, Fut>(
    context: &CookingUpdateContext<'_>,
    active_v1_run_id: Uuid,
    candidates: CookingUpdateCandidates,
    relay_run_id: Uuid,
    relay_rotation: super::super::cooking::CredentialRotation,
    submit_deferred_event: F,
) -> UpdateSequence
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Uuid>,
{
    let migration = begin_update_while_v1_active(
        context,
        candidates.migrate_release_agent_id,
        active_v1_run_id,
        candidates.migrate_rule_ids,
    )
    .await;
    let deferred_event_id = submit_deferred_event().await;
    assert_event_waiting_behind_gate(context.pool, deferred_event_id).await;
    finish_compatible_update(context, migration).await;
    assert_deferred_event_uses_revision(
        context.pool,
        deferred_event_id,
        migration.candidate_revision_id,
        context.timeout,
    )
    .await;
    let migrated_context = CookingUpdateContext {
        pool: context.pool,
        running: context.running,
        instance_id: context.instance_id,
        gateway: context.gateway,
        current_revision_id: migration.candidate_revision_id,
        owner: context.owner,
        rpc_token: context.rpc_token,
        brokered_rule_ids: BrokeredRuleIds {
            model: migration.model_rule_id,
            relay: migration.relay_rule_id,
        },
        timeout: context.timeout,
    };
    exercise_explicit_rollback(
        &migrated_context,
        candidates.rollback_release_agent_id,
        candidates.rollback_rule_ids,
    )
    .await;
    let abnormal = exercise_abnormal_recovery(
        &migrated_context,
        candidates.abnormal_release_agent_id,
        candidates.abnormal_rule_ids,
    )
    .await;
    UpdateSequence {
        migration,
        abnormal,
        relay_run_id,
        relay_rotation,
    }
}

/// Runs the complete update sequence with a real active v1 request held at
/// the model boundary. Request 47 enters before `CreateUpdate`, request 48 is
/// accepted behind the closed gate, and releasing the model response lets the
/// normal v1 run clean up before the migration hook is admitted.
pub(crate) async fn exercise_barrier_update_sequence(
    context: &CookingUpdateContext<'_>,
    upstream: &crate::BrokeredTlsUpstream,
    candidates: CookingUpdateCandidates,
) -> UpdateSequence {
    let public = std::env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("public Caddy URL");
    let url = format!("{public}/gateway/cooking/telegram");
    let client = super::super::cooking::caddy_gateway_client();
    let held = super::super::cooking::send_update(&client, &url, 47, 1001, "stew").await;
    assert_eq!(held.status(), reqwest::StatusCode::OK);
    held.bytes().await.expect("held cooking acknowledgement");
    upstream.wait_update_v1_entered().await;
    let active = super::super::cooking::wait_for_active_event_run(
        context.pool,
        context.gateway.mailbox_id.as_uuid(),
        context.timeout,
        47,
    )
    .await;
    let rotation = rotate_model_credential_while_held(context, active.run_id.as_uuid()).await;
    // Event 46 is the completed relay fault/retry proof.  Keep its original
    // lease as the historical old-version assertion, while event 47 proves
    // that an in-flight v1 run has already issued its relay lease before the
    // source rotates and the candidate rules are cloned.
    let relay_run =
        super::super::cooking::wait_for_event_run(context.pool, context.gateway, 46).await;
    wait_for_active_brokered_lease(
        context.pool,
        active.run_id.as_uuid(),
        super::super::cooking::RELAY_RULE,
        context.timeout,
    )
    .await;
    let relay_rotation = super::super::cooking::rotate_brokered_credential(
        context.pool,
        UserId::from_uuid(context.owner),
        relay_run.run_id.as_uuid(),
        super::super::cooking::RELAY_RULE,
        super::super::cooking::RELAY_ROTATED_SENTINEL,
    )
    .await;
    let sequence = exercise_update_sequence(
        context,
        active.run_id.as_uuid(),
        candidates,
        relay_run.run_id.as_uuid(),
        relay_rotation,
        || async {
            let queued = super::super::cooking::send_update(&client, &url, 48, 1002, "curry").await;
            assert_eq!(queued.status(), reqwest::StatusCode::OK);
            queued
                .bytes()
                .await
                .expect("deferred cooking acknowledgement");
            let event_id = super::super::cooking::wait_for_event_id(
                context.pool,
                context.gateway.mailbox_id.as_uuid(),
                context.timeout,
                48,
            )
            .await;
            assert_event_waiting_behind_gate(context.pool, event_id).await;
            upstream.release_update_v1();
            event_id
        },
    )
    .await;
    let deferred_event_id = super::super::cooking::wait_for_event_id(
        context.pool,
        context.gateway.mailbox_id.as_uuid(),
        context.timeout,
        48,
    )
    .await;
    assert_rotated_model_lease(
        context.pool,
        active.run_id.as_uuid(),
        deferred_event_id,
        sequence.migration.model_rule_id,
        rotation,
    )
    .await;
    super::super::cooking::wait_for_event_run(context.pool, context.gateway, 47).await;
    sequence
}
