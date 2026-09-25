use super::*;

#[allow(clippy::needless_borrow, clippy::too_many_lines)]
pub async fn run_cooking_update_sequence(
    prep: &CookingPreparation,
) -> (Option<cooking_updates::UpdateSequence>, Option<PathBuf>) {
    let pool = &prep.pool;
    let restarted = prep.running.as_ref().expect("running daemon for updates");
    let actual_instance = &prep.actual_instance;
    let actual_fixture = &prep.actual_fixture;
    let rpc_token = prep.rpc_token.as_ref();
    let actual_brokered = &prep.actual_brokered;
    let user_id = prep.user_id;
    let cooking_wait_timeout = prep.cooking_wait_timeout;
    let update_builds = prep.update_builds.as_ref();
    let cooking_update_rule_ids = prep.cooking_update_rule_ids;
    let mut update_state_volume_disk: Option<PathBuf> = None;
    let update_sequence = if let Some(update_builds) = update_builds {
        let current_revision_id: uuid::Uuid =
            sqlx::query_scalar("SELECT active_revision_id FROM agent_instances WHERE id = $1")
                .bind(actual_instance.instance)
                .fetch_one(pool)
                .await
                .expect("load configured cooking revision");
        let sequence = cooking_updates::exercise_barrier_update_sequence(
            &cooking_updates::CookingUpdateContext {
                pool,
                running: &restarted,
                instance_id: actual_instance.instance,
                gateway: &actual_fixture,
                current_revision_id,
                owner: user_id.as_uuid(),
                rpc_token,
                brokered_rule_ids: cooking_updates::BrokeredRuleIds {
                    model: cooking::MODEL_RULE,
                    relay: cooking::RELAY_RULE,
                },
                timeout: cooking_wait_timeout,
            },
            &actual_brokered.upstream,
            cooking_updates::CookingUpdateCandidates {
                migrate_release_agent_id: update_builds.migrate.release_agent_id,
                rollback_release_agent_id: update_builds.rollback.release_agent_id,
                abnormal_release_agent_id: update_builds.abnormal.release_agent_id,
                migrate_rule_ids: cooking_update_rule_ids
                    .expect("update rule IDs allocated with update builds")
                    .0,
                rollback_rule_ids: cooking_update_rule_ids
                    .expect("update rule IDs allocated with update builds")
                    .1,
                abnormal_rule_ids: cooking_update_rule_ids
                    .expect("update rule IDs allocated with update builds")
                    .2,
            },
        )
        .await;
        let disk: String = sqlx::query_scalar(
            "SELECT host_path FROM agent_instance_state_volumes
             WHERE instance_id = $1",
        )
        .bind(actual_instance.instance)
        .fetch_one(pool)
        .await
        .expect("load cooking state volume disk path");
        update_state_volume_disk = Some(PathBuf::from(disk));
        Some(sequence)
    } else {
        None
    };

    (update_sequence, update_state_volume_disk)
}

#[allow(clippy::too_many_arguments)]
pub async fn run_cooking_updates(
    prep: &mut CookingPreparation,
    adversarial_probe: cooking_adversarial_agent::AdversarialAgentProbe,
    crash_agent_build: cooking_builds::PublishedCookingRepository,
    crash_instance: cooking_adversarial_agent::PreparedBrokeredInstance,
    crash_disk: PathBuf,
) {
    let (update_sequence, update_state_volume_disk) = run_cooking_update_sequence(prep).await;
    if let Some(sequence) = update_sequence.as_ref() {
        run_cooking_rotation_phase(prep, sequence).await;
    }
    if prep.browser_e2e {
        run_cooking_browser_phase(prep, update_sequence.as_ref()).await;
    }
    run_cooking_cleanup_phase(
        prep,
        update_sequence,
        update_state_volume_disk,
        adversarial_probe,
        crash_agent_build,
        crash_instance,
        crash_disk,
    )
    .await;
}
