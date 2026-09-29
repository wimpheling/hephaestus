//! Update hook result recording, reconciliation, and activation.

use super::{
    AgentUpdateId, ReleaseService, ReleaseServiceError, RunId, UpdateDecision, UpdateHookResult,
    UpdateLifecycleRow, UpdateRunResultRow, append_event, append_rejected_update_event,
    append_uncertain_update_events, enqueue_mailbox_wakes, existing_terminal_decision,
    mark_volume_lease, materialize_deferred_triggers, reopen_after_update,
};
use serde_json::json;

impl ReleaseService {
    /// Records the hook terminal contract. Exit zero is the irreversible
    /// commit point but activation is a separate reconciliable transaction.
    ///
    /// # Errors
    ///
    /// Fails for stale lifecycle or database errors.
    #[tracing::instrument(skip_all, fields(%update_id))]
    pub async fn record_update_hook_result(
        &self,
        update_id: AgentUpdateId,
        result: UpdateHookResult,
    ) -> Result<UpdateDecision, ReleaseServiceError> {
        let mut tx = self.pool.begin().await?;
        let update: UpdateLifecycleRow = sqlx::query_as(
            "SELECT update.instance_id, update.expected_current_revision_id,
                    update.candidate_revision_id, update.state
             FROM agent_updates AS update
             JOIN agent_instances AS instance ON instance.id = update.instance_id
             WHERE update.id = $1
             FOR UPDATE OF update, instance",
        )
        .bind(update_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        if update.state != "hook_running" {
            return existing_terminal_decision(&update.state);
        }
        let decision = match result {
            UpdateHookResult::Committed => {
                sqlx::query(
                    "UPDATE agent_updates
                     SET state = 'hook_committed', hook_exit_code = 0,
                         updated_at = now()
                     WHERE id = $1",
                )
                .bind(update_id.as_uuid())
                .execute(&mut *tx)
                .await?;
                append_event(
                    &mut tx,
                    update_id.as_uuid(),
                    "hephaestus.agent_update.hook_committed.v1",
                    "agent_update.hook_committed.v1",
                    json!({"schema_version": 1, "update_id": update_id}),
                )
                .await?;
                UpdateDecision::ActivationRecovery
            }
            UpdateHookResult::Rejected(exit_code) => {
                if exit_code == 0 {
                    return Err(ReleaseServiceError::InvalidHookResult);
                }
                sqlx::query(
                    "UPDATE agent_updates
                     SET state = 'rejected', hook_exit_code = $2,
                         final_decision = 'agent_rejected',
                         completed_at = now(), updated_at = now()
                     WHERE id = $1",
                )
                .bind(update_id.as_uuid())
                .bind(exit_code)
                .execute(&mut *tx)
                .await?;
                reopen_after_update(
                    &mut tx,
                    update_id,
                    update.instance_id,
                    update.expected_current_revision_id,
                    "update_rejected",
                )
                .await?;
                append_rejected_update_event(&mut tx, update_id, &update, exit_code).await?;
                UpdateDecision::AgentRejected
            }
            UpdateHookResult::Uncertain => {
                sqlx::query(
                    "UPDATE agent_updates
                     SET state = 'compatibility_unknown',
                         final_decision = 'unknown',
                         completed_at = now(), updated_at = now()
                     WHERE id = $1",
                )
                .bind(update_id.as_uuid())
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agent_instances
                     SET state = 'paused_unknown_state', run_gate_open = false,
                         version = version + 1, updated_at = now()
                     WHERE id = $1",
                )
                .bind(update.instance_id)
                .execute(&mut *tx)
                .await?;
                mark_volume_lease(&mut tx, update_id, "recovery_required").await?;
                append_uncertain_update_events(
                    &mut tx,
                    update_id,
                    &update,
                    "hook_result_uncertain",
                    "paused_unknown_state",
                )
                .await?;
                UpdateDecision::CompatibilityUnknown
            }
        };
        tx.commit().await?;
        Ok(decision)
    }

    /// Reconciles one cleaned update run into the durable update state machine.
    ///
    /// Successful hooks are activated immediately; explicit nonzero exits
    /// reopen the prior revision; every ambiguous terminal path pauses the
    /// instance for recovery. Repeated calls are idempotent.
    ///
    /// # Errors
    ///
    /// Returns a stable lifecycle error until the exact hook run is cleaned,
    /// or a database error when its result cannot be persisted.
    #[tracing::instrument(skip_all, fields(%run_id))]
    pub async fn reconcile_update_run(
        &self,
        run_id: RunId,
    ) -> Result<UpdateDecision, ReleaseServiceError> {
        let row: UpdateRunResultRow = sqlx::query_as(
            "SELECT update.id AS update_id, update.state AS update_state,
                    run.state AS run_state, run.outcome, run.exit_code,
                    run.exit_signal, run.failure
             FROM agent_updates AS update
             JOIN runs AS run ON run.id = update.hook_run_id
             WHERE update.hook_run_id = $1 AND run.run_kind = 'update'",
        )
        .bind(run_id.as_uuid())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        let update_id = AgentUpdateId::from_uuid(row.update_id);
        match row.update_state.as_str() {
            "hook_committed" => return self.activate_committed_update(update_id).await,
            "hook_running" => {}
            state => return existing_terminal_decision(state),
        }
        if row.run_state != "cleaned_up" {
            return Err(ReleaseServiceError::InvalidUpdateLifecycle);
        }
        let result = match (
            row.outcome.as_deref(),
            row.exit_code,
            row.exit_signal,
            row.failure.as_deref(),
        ) {
            (Some("succeeded"), Some(0), None, _) => UpdateHookResult::Committed,
            (Some("failed"), Some(code), None, None) if code != 0 => {
                UpdateHookResult::Rejected(code)
            }
            (Some("failed" | "cancelled"), _, _, _) => UpdateHookResult::Uncertain,
            _ => return Err(ReleaseServiceError::InvalidHookResult),
        };
        let decision = self.record_update_hook_result(update_id, result).await?;
        if result == UpdateHookResult::Committed {
            self.activate_committed_update(update_id).await
        } else {
            Ok(decision)
        }
    }

    /// Activates an exact hook-committed candidate without re-running the hook
    /// or allowing later source revocation to veto the committed migration.
    ///
    /// # Errors
    ///
    /// Returns database failures; CAS anomalies are durably converted to
    /// activation-recovery state and returned as a decision.
    #[tracing::instrument(skip_all, fields(%update_id))]
    pub async fn activate_committed_update(
        &self,
        update_id: AgentUpdateId,
    ) -> Result<UpdateDecision, ReleaseServiceError> {
        let mut tx = self.pool.begin().await?;
        let update: UpdateLifecycleRow = sqlx::query_as(
            "SELECT update.instance_id, update.expected_current_revision_id,
                    update.candidate_revision_id, update.state
             FROM agent_updates AS update
             JOIN agent_instances AS instance ON instance.id = update.instance_id
             WHERE update.id = $1
             FOR UPDATE OF update, instance",
        )
        .bind(update_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        if update.state == "activated" {
            tx.commit().await?;
            return Ok(UpdateDecision::Activated);
        }
        if update.state != "hook_committed" {
            return Err(ReleaseServiceError::InvalidUpdateLifecycle);
        }
        let activated = sqlx::query(
            "UPDATE agent_instances
             SET active_revision_id = $3, state = 'active',
                 run_gate_open = true, version = version + 1,
                 updated_at = now()
             WHERE id = $1 AND active_revision_id = $2
               AND state = 'updating' AND NOT run_gate_open",
        )
        .bind(update.instance_id)
        .bind(update.expected_current_revision_id)
        .bind(update.candidate_revision_id)
        .execute(&mut *tx)
        .await?;
        if activated.rows_affected() != 1 {
            sqlx::query(
                "UPDATE agent_updates
                 SET state = 'activation_recovery',
                     final_decision = 'recovery', updated_at = now()
                 WHERE id = $1",
            )
            .bind(update_id.as_uuid())
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE agent_instances
                 SET state = 'paused_activation_recovery',
                     run_gate_open = false, version = version + 1,
                     updated_at = now()
                 WHERE id = $1",
            )
            .bind(update.instance_id)
            .execute(&mut *tx)
            .await?;
            mark_volume_lease(&mut tx, update_id, "recovery_required").await?;
            append_uncertain_update_events(
                &mut tx,
                update_id,
                &update,
                "activation_compare_and_swap_failed",
                "paused_activation_recovery",
            )
            .await?;
            tx.commit().await?;
            return Ok(UpdateDecision::ActivationRecovery);
        }
        sqlx::query(
            "UPDATE agent_updates
             SET state = 'activated', final_decision = 'activated',
                 completed_at = now(), updated_at = now()
             WHERE id = $1",
        )
        .bind(update_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        mark_volume_lease(&mut tx, update_id, "released").await?;
        materialize_deferred_triggers(&mut tx, update.instance_id, update.candidate_revision_id)
            .await?;
        enqueue_mailbox_wakes(&mut tx, update.instance_id).await?;
        append_event(
            &mut tx,
            update_id.as_uuid(),
            "hephaestus.agent_update.completed.v1",
            "agent_update.completed.v1",
            json!({
                "update_id": update_id,
                "instance_id": update.instance_id,
                "revision_id": update.candidate_revision_id,
                "decision": "activated",
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(UpdateDecision::Activated)
    }
}
