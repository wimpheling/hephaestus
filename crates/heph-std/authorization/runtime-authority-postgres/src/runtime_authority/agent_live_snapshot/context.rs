//! Exact owned producer/source phase evidence. This query never issues authority.

use runtime_authority::{LiveRunSnapshotRequest, RuntimeAuthorityError};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::super::common::storage;

#[derive(sqlx::FromRow)]
pub struct Source {
    pub actor_id: Uuid,
    pub request_id: Uuid,
}

pub async fn source(
    tx: &mut Transaction<'_, Postgres>,
    request: &LiveRunSnapshotRequest,
) -> Result<Source, RuntimeAuthorityError> {
    let pins = request.pins();
    sqlx::query_as(
        "SELECT CASE WHEN run.run_kind='normal' THEN request.actor_id ELSE admission.actor_id END AS actor_id,
                CASE WHEN run.run_kind='normal' THEN request.request_id ELSE admission.request_id END AS request_id
         FROM runs run
         JOIN agent_instances instance ON instance.id=run.instance_id
         JOIN agent_instance_revisions revision
           ON revision.id=run.instance_revision_id AND revision.instance_id=run.instance_id
          AND revision.release_agent_id=run.release_agent_id
         JOIN release_agents agent ON agent.id=run.release_agent_id AND agent.release_id=run.release_id
         JOIN releases release ON release.id=run.release_id
         LEFT JOIN run_requests request
           ON request.run_id=run.id AND request.command_id=run.command_id
          AND ROW(request.instance_id,request.instance_revision_id,request.release_id,
                  request.release_agent_id,request.attachment_id,request.requires_state)
              IS NOT DISTINCT FROM ROW(run.instance_id,run.instance_revision_id,run.release_id,
                  run.release_agent_id,run.attachment_id,run.requires_state)
         LEFT JOIN run_vm_plan_admissions admission
           ON admission.run_id=run.id AND admission.command_id=run.command_id
          AND admission.instance_id=run.instance_id AND admission.instance_revision_id=run.instance_revision_id
          AND admission.consumption_kind='provision' AND admission.consumed_at IS NOT NULL
          AND ROW(admission.vm_id,admission.vm_provider_namespace,admission.vm_host_id)
              IS NOT DISTINCT FROM ROW(run.vm_id,run.vm_provider_namespace,run.vm_host_id)
         WHERE run.id=$1 AND run.instance_id=$2 AND run.instance_revision_id=$3
           AND run.release_id=$4 AND run.release_agent_id=$5 AND run.command_id=$6
           AND run.execution_profile='owned_canonical'
           AND run.vm_id=run.id::text AND run.vm_binding_kind='planned'
           AND run.vm_host_id IS NOT NULL AND run.vm_provider_namespace IS NOT NULL
           AND run.cancel_requested_at IS NULL
           AND NOT EXISTS(SELECT 1 FROM agent_instance_removal_requests removal WHERE removal.instance_id=instance.id)
           AND revision.runnable AND release.state='published' AND agent.requires_state=run.requires_state
           AND (($7 AND run.state IN ('provisioning','starting','running','succeeded','failed','cleaning_up'))
             OR (NOT $7 AND run.state IN ('leasing_volume','provisioning')
                 AND run.volume_acquisition_closed_at IS NULL))
           AND NOT EXISTS(SELECT 1 FROM mailbox_delivery_attempts WHERE run_id=run.id)
           AND ((run.run_kind='normal' AND instance.state IN ('active','update_rejected')
                 AND instance.active_revision_id=revision.id AND instance.run_gate_open
                 AND request.actor_id IS NOT NULL AND request.request_id IS NOT NULL
                 AND request.request_kind='instance_normal'
                 AND EXISTS(SELECT 1 FROM agent_attachments attachment
                     WHERE attachment.id=run.attachment_id AND attachment.instance_id=instance.id
                       AND attachment.enabled AND attachment.removed_at IS NULL)
                 AND EXISTS(SELECT 1 FROM command_inbox inbox WHERE inbox.command_id=run.command_id
                     AND inbox.command_type='start_run'
                     AND ROW(inbox.payload->>'command_id',inbox.payload->>'run_id',inbox.payload->>'instance_id',
                         inbox.payload->>'instance_revision_id',inbox.payload->>'release_id',inbox.payload->>'release_agent_id',
                         inbox.payload->>'attachment_id',inbox.payload->>'kind',inbox.payload->'requires_state')
                       IS NOT DISTINCT FROM ROW(run.command_id::text,run.id::text,run.instance_id::text,
                         run.instance_revision_id::text,run.release_id::text,run.release_agent_id::text,
                         run.attachment_id::text,run.run_kind,to_jsonb(run.requires_state))))
             OR (run.run_kind='update' AND instance.state='updating'
                 AND admission.actor_id IS NOT NULL AND admission.request_id IS NOT NULL
                 AND EXISTS(SELECT 1 FROM agent_updates update
                     JOIN release_command_inbox command ON command.operation='begin_update_hook'
                       AND command.aggregate_id=update.id AND command.secondary_id=run.id
                     WHERE update.id=admission.update_id AND update.hook_run_id=run.id
                       AND update.instance_id=instance.id AND update.candidate_revision_id=revision.id
                       AND command.command_key=admission.producer_command_key
                       AND command.actor_id=admission.actor_id AND command.request_id=admission.request_id)))
           AND NOT EXISTS(SELECT 1 FROM release_capability_requirements requirement
               WHERE requirement.release_agent_id=agent.id AND requirement.slot_required
                 AND NOT EXISTS(SELECT 1 FROM agent_capability_bindings binding
                     WHERE binding.instance_revision_id=revision.id AND binding.requirement_id=requirement.id))",
    )
    .bind(pins.run()).bind(pins.instance()).bind(pins.revision())
    .bind(pins.release()).bind(pins.release_agent()).bind(pins.command())
    .bind(request.ceiling().is_some())
    .fetch_optional(&mut **tx).await.map_err(storage)?
    .ok_or(RuntimeAuthorityError::Unsupported)
}

pub async fn ensure_worker(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<(), RuntimeAuthorityError> {
    let worker: bool = sqlx::query_scalar("SELECT current_user='hephaestus_worker'")
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?;
    if worker {
        Ok(())
    } else {
        Err(RuntimeAuthorityError::Unsupported)
    }
}
