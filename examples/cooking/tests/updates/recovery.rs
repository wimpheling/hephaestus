// Reuse the update facade imports across focused lifecycle phases.
#[allow(unused_imports)]
use super::*;
/// Completes a previously admitted compatible migration candidate.
pub(crate) async fn finish_compatible_update(context: &CookingUpdateContext<'_>, ids: UpdateIds) {
    wait_for_state(context, ids.update_id, "activated").await;
    let state = load_state(context.pool, ids.update_id).await;
    assert_eq!(state.instance, "active");
    assert!(state.run_gate_open);
    assert_eq!(state.active_revision_id, Some(ids.candidate_revision_id));
    assert_candidate_brokered_rules(context.pool, ids).await;
}

/// Confirms that the production update transaction carried both explicit
/// candidate rules and that its ordinary parameters select those same IDs.
/// This is intentionally a projection check: the persistence regression owns
/// the full canonical-hash/source-immutability proof.
pub(crate) async fn assert_candidate_brokered_rules(pool: &PgPool, ids: UpdateIds) {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT rule.id, binding.slot_key
           FROM brokered_secret_rules AS rule
           JOIN agent_secret_bindings AS binding
             ON binding.id = rule.binding_id
            AND binding.instance_revision_id = rule.instance_revision_id
          WHERE rule.instance_revision_id = $1
          ORDER BY binding.slot_key",
    )
    .bind(ids.candidate_revision_id)
    .fetch_all(pool)
    .await
    .expect("candidate brokered rule projection");
    assert_eq!(
        rows,
        vec![
            (ids.model_rule_id, String::from("model")),
            (ids.relay_rule_id, String::from("telegram_relay")),
        ],
        "candidate revision has exactly the declared model and relay rules"
    );
    let parameters: serde_json::Value =
        sqlx::query_scalar("SELECT parameters FROM agent_instance_revisions WHERE id = $1")
            .bind(ids.candidate_revision_id)
            .fetch_one(pool)
            .await
            .expect("candidate cooking parameters");
    let model_rule_id = ids.model_rule_id.to_string();
    let relay_rule_id = ids.relay_rule_id.to_string();
    assert_eq!(
        parameters["model_rule_id"].as_str(),
        Some(model_rule_id.as_str())
    );
    assert_eq!(
        parameters["relay_rule_id"].as_str(),
        Some(relay_rule_id.as_str())
    );
}

/// Drives a candidate whose update hook exits nonzero.  This is the explicit
/// agent rollback contract: the prior revision remains active and the update
/// is rejected without claiming to roll back agent-owned state.
pub(crate) async fn exercise_explicit_rollback(
    context: &CookingUpdateContext<'_>,
    candidate_release_agent_id: Uuid,
    candidate_rule_ids: BrokeredRuleIds,
) -> UpdateIds {
    let ids = create_update(context, candidate_release_agent_id, candidate_rule_ids).await;
    wait_for_state(context, ids.update_id, "rejected").await;
    let state = load_state(context.pool, ids.update_id).await;
    assert_eq!(state.instance, "update_rejected");
    assert!(state.run_gate_open);
    assert_eq!(state.active_revision_id, Some(context.current_revision_id));
    assert_update_event_order(context.pool, ids.update_id, "agent_update.rejected.v1").await;
    ids
}

/// Drives a signal-terminated or otherwise ambiguous hook to the
/// compatibility-unknown state, then performs the authorized reject action.
/// The assertion intentionally makes no claim about guest-owned rollback.
pub(crate) async fn exercise_abnormal_recovery(
    context: &CookingUpdateContext<'_>,
    candidate_release_agent_id: Uuid,
    candidate_rule_ids: BrokeredRuleIds,
) -> UpdateIds {
    let ids =
        exercise_abnormal_pending(context, candidate_release_agent_id, candidate_rule_ids).await;
    recover_update(context, ids.update_id, "RECOVERY_ACTION_REJECT").await;
    wait_for_state(context, ids.update_id, "rejected").await;
    let recovered = load_state(context.pool, ids.update_id).await;
    assert_eq!(recovered.instance, "update_rejected");
    assert!(recovered.run_gate_open);
    assert_eq!(
        recovered.active_revision_id,
        Some(context.current_revision_id)
    );
    assert_update_event_order(context.pool, ids.update_id, "agent_update.uncertain.v1").await;
    ids
}

/// Creates the abnormal update and leaves it in its paused compatibility
/// state for the browser recovery phase to resolve through the UI.
pub(crate) async fn exercise_abnormal_pending(
    context: &CookingUpdateContext<'_>,
    candidate_release_agent_id: Uuid,
    candidate_rule_ids: BrokeredRuleIds,
) -> UpdateIds {
    let ids = create_update(context, candidate_release_agent_id, candidate_rule_ids).await;
    wait_for_state(context, ids.update_id, "compatibility_unknown").await;
    let paused = load_state(context.pool, ids.update_id).await;
    assert_eq!(paused.instance, "paused_unknown_state");
    assert!(!paused.run_gate_open);
    assert_eq!(paused.active_revision_id, Some(context.current_revision_id));
    ids
}

/// Verifies that a mailbox event accepted behind the closed gate eventually
/// runs against the newly active revision.  The event ID must come from the
/// real gateway publication, so a hand-built deferred row cannot satisfy this
/// check.  Call [`assert_event_waiting_behind_gate`] before reopening the gate
/// when the pre-activation pending state also needs to be recorded.
pub(crate) async fn assert_deferred_event_uses_revision(
    pool: &PgPool,
    event_id: Uuid,
    candidate_revision_id: Uuid,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    loop {
        let row: Option<DeferredEventProjection> = sqlx::query_as(
            "SELECT delivery.disposition, attempt.run_id,
                    run.instance_revision_id, run.state, run.outcome
               FROM mailbox_deliveries delivery
               LEFT JOIN mailbox_delivery_attempts attempt
                 ON attempt.event_id = delivery.event_id
               LEFT JOIN runs run ON run.id = attempt.run_id
              WHERE delivery.event_id = $1
              ORDER BY attempt.attempt_number DESC NULLS LAST
              LIMIT 1",
        )
        .bind(event_id)
        .fetch_optional(pool)
        .await
        .expect("deferred cooking event projection");
        if let Some((disposition, Some(run_id), revision_id, run_state, outcome)) = &row {
            assert_eq!(*revision_id, Some(candidate_revision_id));
            assert_ne!(*run_id, Uuid::nil());
            if run_state.as_deref() == Some("cleaned_up") {
                assert_eq!(
                    outcome.as_deref(),
                    Some("succeeded"),
                    "deferred mailbox run reached terminal failure: disposition={disposition}"
                );
            }
            if row.as_ref().is_some_and(|projection| {
                deferred_event_is_terminal_success(projection, candidate_revision_id)
            }) {
                return;
            }
            assert!(
                ["pending", "eligible", "leased", "running", "retryable"]
                    .contains(&disposition.as_str()),
                "deferred mailbox event has unexpected nonterminal disposition {disposition}"
            );
        }
        assert!(
            Instant::now() < deadline,
            "deferred event did not materialize against candidate revision"
        );
        sleep(Duration::from_millis(50)).await;
    }
}

/// Asserts the pre-activation half of the mailbox barrier.  A normal cooking
/// event accepted while the instance gate is closed has a durable delivery,
/// but no delivery attempt or run yet.
pub(crate) async fn assert_event_waiting_behind_gate(pool: &PgPool, event_id: Uuid) {
    let row: (String, i64) = sqlx::query_as(
        "SELECT delivery.disposition,
                (SELECT count(*) FROM mailbox_delivery_attempts attempt
                  WHERE attempt.event_id = delivery.event_id)
           FROM mailbox_deliveries delivery
          WHERE delivery.event_id = $1",
    )
    .bind(event_id)
    .fetch_one(pool)
    .await
    .expect("mailbox event barrier projection");
    assert!(
        ["pending", "eligible", "retryable"].contains(&row.0.as_str()),
        "event accepted behind a closed gate must remain dispatchable: {}",
        row.0
    );
    assert_eq!(row.1, 0, "gated event must not start a pre-update run");
}
