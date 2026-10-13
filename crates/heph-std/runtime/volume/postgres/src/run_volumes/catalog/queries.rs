use runtime_types::RunId;
use sqlx::{Postgres, Transaction};
use volume_trait::VolumeError;

use super::Execution;
use crate::errors::metadata;

pub(super) async fn ordinary(
    tx: &mut Transaction<'_, Postgres>,
    run_id: RunId,
) -> Result<Execution, VolumeError> {
    sqlx::query_as(
        "SELECT run.instance_id,run.instance_revision_id,run.release_id,run.release_agent_id,
         instance.project_id,instance.state_volume_id,instance.volume_mode,agent.requires_state,
         agent.runtime_contract,agent.runtime_contract_hash,run.volume_selection_snapshot,
         (run.volume_acquisition_closed_at IS NULL AND run.cancel_requested_at IS NULL
          AND run.state IN ('leasing_volume','provisioning','starting','running')
          AND release.state='published'
          AND NOT EXISTS(SELECT 1 FROM agent_instance_removal_requests removal WHERE removal.instance_id=instance.id)
          AND ((run.run_kind='normal' AND instance.state IN ('active','update_rejected')
                AND instance.run_gate_open AND instance.active_revision_id=revision.id AND revision.runnable
                AND (instance.volume_mode='legacy' OR named_volume_dispatch_supported(instance.id)))
            OR (run.run_kind='update' AND instance.volume_mode='legacy' AND instance.state='updating'
                AND NOT instance.run_gate_open AND EXISTS(SELECT 1 FROM agent_updates candidate
                    WHERE candidate.instance_id=instance.id AND candidate.candidate_revision_id=revision.id
                    AND candidate.hook_run_id=run.id AND candidate.state='hook_running')))) AS admitted
         FROM runs run JOIN agent_instances instance ON instance.id=run.instance_id
         JOIN agent_instance_revisions revision ON revision.id=run.instance_revision_id AND revision.instance_id=instance.id
             AND revision.release_agent_id=run.release_agent_id
         JOIN release_agents agent ON agent.id=run.release_agent_id AND agent.release_id=run.release_id
         JOIN releases release ON release.id=run.release_id WHERE run.id=$1",
        ).bind(run_id.as_uuid()).fetch_optional(&mut **tx).await.map_err(metadata)?
        .ok_or(VolumeError::InvalidState("run has no exact released revision"))
}

pub(super) async fn invocation(
    tx: &mut Transaction<'_, Postgres>,
    run_id: RunId,
) -> Result<Execution, VolumeError> {
    sqlx::query_as(
        "SELECT run.instance_id,run.instance_revision_id,run.release_id,run.release_agent_id,
         instance.project_id,instance.state_volume_id,instance.volume_mode,agent.requires_state,
         agent.runtime_contract,agent.runtime_contract_hash,run.volume_selection_snapshot,
         (run.volume_acquisition_closed_at IS NULL AND run.cancel_requested_at IS NULL
          AND run.state IN ('leasing_volume','provisioning','starting','running')
          AND release.state='published'
          AND NOT EXISTS(SELECT 1 FROM agent_instance_removal_requests removal WHERE removal.instance_id=instance.id)
          AND ((run.run_kind='normal' AND instance.state IN ('active','update_rejected')
                AND instance.run_gate_open AND instance.active_revision_id=revision.id AND revision.runnable
                AND (instance.volume_mode='legacy' OR named_volume_dispatch_supported(instance.id)))
            OR (run.run_kind='update' AND instance.volume_mode='legacy' AND instance.state='updating'
                AND NOT instance.run_gate_open AND EXISTS(SELECT 1 FROM agent_updates candidate
                    WHERE candidate.instance_id=instance.id AND candidate.candidate_revision_id=revision.id
                    AND candidate.hook_run_id=run.id AND candidate.state='hook_running'))
            OR (run.run_kind='invocation' AND invocation119_open_set(run.id)))) AS admitted
         FROM runs run JOIN agent_instances instance ON instance.id=run.instance_id
         JOIN agent_instance_revisions revision ON revision.id=run.instance_revision_id AND revision.instance_id=instance.id
             AND revision.release_agent_id=run.release_agent_id
         JOIN release_agents agent ON agent.id=run.release_agent_id AND agent.release_id=run.release_id
         JOIN releases release ON release.id=run.release_id WHERE run.id=$1",
        ).bind(run_id.as_uuid()).fetch_optional(&mut **tx).await.map_err(metadata)?
        .ok_or(VolumeError::InvalidState("run has no exact released revision"))
}
