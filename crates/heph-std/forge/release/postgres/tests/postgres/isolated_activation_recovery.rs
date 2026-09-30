use super::*;

/// Completes the activation recovery CAS path and returns the active revision.
///
/// # Panics
/// Panics when a phase fixture operation or durable assertion fails.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn prepare(context: IsolatedRecoveryContext) -> IsolatedAttachmentContext {
    let IsolatedRecoveryContext {
        pool,
        fixture,
        service,
        actor,
        release_id,
        first_instance,
        second_instance,
        update_release_agent,
        update_release_id,
        update_candidate_revision,
        ..
    } = context;
    let resume_update_id = AgentUpdateId::new();
    let resume_candidate = AgentInstanceRevisionId::new();
    service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("resume-update", resume_update_id.as_uuid()),
                update_id: resume_update_id,
                instance_id: first_instance,
                expected_revision_id: update_candidate_revision,
                candidate_revision_id: resume_candidate,
                candidate_release_agent_id: update_release_agent,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                brokered_rule_copies: Vec::new(),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v3"),
            },
        )
        .await
        .expect("activation-recovery candidate");
    service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("resume-hook", resume_update_id.as_uuid()),
                update_id: resume_update_id,
                hook_run_id: RunId::new(),
            },
        )
        .await
        .expect("activation-recovery hook");
    service
        .record_update_hook_result(resume_update_id, UpdateHookResult::Committed)
        .await
        .expect("durable hook commit");
    service
        .revoke(
            &actor,
            key("revoke-release", update_release_id),
            ReleaseId::from_uuid(update_release_id),
        )
        .await
        .expect("revocation after the hook commit point");
    sqlx::query("UPDATE agent_instances SET state = 'recovering' WHERE id = $1")
        .bind(first_instance.as_uuid())
        .execute(&pool)
        .await
        .expect("simulate activation CAS anomaly");
    assert_eq!(
        service
            .activate_committed_update(resume_update_id)
            .await
            .expect("activation anomaly becomes recovery"),
        UpdateDecision::ActivationRecovery
    );
    assert_eq!(
        service
            .recover_update(
                &actor,
                RecoverInstanceUpdate {
                    command_key: key("resume-recovery", resume_update_id.as_uuid()),
                    update_id: resume_update_id,
                    action: UpdateRecoveryAction::ResumeActivation,
                },
            )
            .await
            .expect("operator resumes durable activation"),
        UpdateRecoveryDecision::CandidateActivated
    );
    let resumed: (Uuid, String, bool) = sqlx::query_as(
        "SELECT active_revision_id, state, run_gate_open
         FROM agent_instances WHERE id = $1",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("resumed candidate");
    assert_eq!(
        resumed,
        (resume_candidate.as_uuid(), String::from("active"), true)
    );

    IsolatedAttachmentContext {
        pool,
        fixture,
        service,
        actor,
        release_id,
        first_instance,
        second_instance,
        update_release_agent,
        update_release_id,
        resume_candidate,
    }
}
