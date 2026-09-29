use super::*;

/// Verifies agent rejection and uncertain update recovery decisions.
///
/// # Panics
/// Panics when a phase fixture operation or durable assertion fails.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn prepare(context: IsolatedRecoveryContext) -> IsolatedRecoveryContext {
    let IsolatedRecoveryContext {
        pool,
        fixture,
        service,
        actor,
        release_id,
        first_instance,
        second_instance,
        revised_first,
        update_release_agent,
        update_release_id,
        update_id,
        update_candidate_revision,
    } = context;
    let rejected_update_id = AgentUpdateId::new();
    service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("agent-rejected-update", rejected_update_id.as_uuid()),
                update_id: rejected_update_id,
                instance_id: first_instance,
                expected_revision_id: update_candidate_revision,
                candidate_revision_id: AgentInstanceRevisionId::new(),
                candidate_release_agent_id: update_release_agent,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("error")),
                )]),
                brokered_rule_copies: Vec::new(),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v3"),
            },
        )
        .await
        .expect("agent-rejected candidate");
    service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("agent-rejected-hook", rejected_update_id.as_uuid()),
                update_id: rejected_update_id,
                hook_run_id: RunId::new(),
            },
        )
        .await
        .expect("agent-rejected hook");
    assert_eq!(
        service
            .record_update_hook_result(rejected_update_id, UpdateHookResult::Rejected(23))
            .await
            .expect("explicit agent rollback result"),
        UpdateDecision::AgentRejected
    );
    let agent_rejected: (Uuid, String, bool, i32) = sqlx::query_as(
        "SELECT instance.active_revision_id, instance.state,
                instance.run_gate_open, update.hook_exit_code
         FROM agent_updates AS update
         JOIN agent_instances AS instance ON instance.id = update.instance_id
         WHERE update.id = $1",
    )
    .bind(rejected_update_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("agent rejection state");
    assert_eq!(
        agent_rejected,
        (
            update_candidate_revision.as_uuid(),
            String::from("update_rejected"),
            true,
            23,
        )
    );
    let rejected_event: bool = sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1 FROM application_events
             WHERE aggregate_type = 'agent_instance' AND aggregate_id = $1
               AND event_type = 'agent_instance.changed'
               AND safe_state = 'rejected'
         )",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("canonical update rejection event");
    assert!(rejected_event);

    let retry_update_id = AgentUpdateId::new();
    let retry_candidate = AgentInstanceRevisionId::new();
    service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("retry-update", retry_update_id.as_uuid()),
                update_id: retry_update_id,
                instance_id: first_instance,
                expected_revision_id: update_candidate_revision,
                candidate_revision_id: retry_candidate,
                candidate_release_agent_id: update_release_agent,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("error")),
                )]),
                brokered_rule_copies: Vec::new(),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v3"),
            },
        )
        .await
        .expect("second update candidate");
    let retry_first_run = RunId::new();
    service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("retry-first-hook", retry_update_id.as_uuid()),
                update_id: retry_update_id,
                hook_run_id: retry_first_run,
            },
        )
        .await
        .expect("first uncertain attempt");
    sqlx::query(
        "UPDATE runs
         SET state = 'cleaned_up', outcome = 'failed', exit_signal = 9,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(retry_first_run.as_uuid())
    .execute(&pool)
    .await
    .expect("persist signal-terminated update run");
    assert_eq!(
        service
            .reconcile_update_run(retry_first_run)
            .await
            .expect("signal failure pauses uncertain update"),
        UpdateDecision::CompatibilityUnknown
    );
    let uncertain_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE aggregate_type = 'agent_instance' AND aggregate_id = $1
           AND event_type = 'agent_instance.changed'
           AND safe_state = 'paused'",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("canonical uncertain update events");
    assert!(uncertain_events >= 2);
    let retry_command_key = key("retry-recovery", retry_update_id.as_uuid());
    assert_eq!(
        service
            .recover_update(
                &actor,
                RecoverInstanceUpdate {
                    command_key: retry_command_key,
                    update_id: retry_update_id,
                    action: UpdateRecoveryAction::RetryHook,
                },
            )
            .await
            .expect("operator-authorized retry"),
        UpdateRecoveryDecision::HookRetryScheduled
    );
    assert_eq!(
        service
            .recover_update(
                &actor,
                RecoverInstanceUpdate {
                    command_key: retry_command_key,
                    update_id: retry_update_id,
                    action: UpdateRecoveryAction::RetryHook,
                },
            )
            .await
            .expect("retry recovery command is idempotent"),
        UpdateRecoveryDecision::HookRetryScheduled
    );
    service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("retry-second-hook", retry_update_id.as_uuid()),
                update_id: retry_update_id,
                hook_run_id: RunId::new(),
            },
        )
        .await
        .expect("retry uses the same update identity");
    service
        .record_update_hook_result(retry_update_id, UpdateHookResult::Uncertain)
        .await
        .expect("second uncertain attempt");
    assert_eq!(
        service
            .recover_update(
                &actor,
                RecoverInstanceUpdate {
                    command_key: key("reject-recovery", retry_update_id.as_uuid()),
                    update_id: retry_update_id,
                    action: UpdateRecoveryAction::RejectCandidate,
                },
            )
            .await
            .expect("operator rejects uncertain candidate"),
        UpdateRecoveryDecision::CandidateRejected
    );
    let rejected_recovery: (Uuid, String, bool, String) = sqlx::query_as(
        "SELECT instance.active_revision_id, instance.state,
                instance.run_gate_open, update.final_decision
         FROM agent_updates AS update
         JOIN agent_instances AS instance ON instance.id = update.instance_id
         WHERE update.id = $1",
    )
    .bind(retry_update_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("rejected recovery state");
    assert_eq!(
        rejected_recovery,
        (
            update_candidate_revision.as_uuid(),
            String::from("update_rejected"),
            true,
            String::from("recovery"),
        )
    );

    IsolatedRecoveryContext {
        pool,
        fixture,
        service,
        actor,
        release_id,
        first_instance,
        second_instance,
        revised_first,
        update_release_agent,
        update_release_id,
        update_id,
        update_candidate_revision,
    }
}
