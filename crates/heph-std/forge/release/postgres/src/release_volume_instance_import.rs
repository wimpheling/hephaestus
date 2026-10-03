//! Atomic initial import of exact named-volume bindings and authority.

use authz_domain::{ObjectRef, ObjectType, Permission};
use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use release_domain::AgentInstanceId;
use release_service::ImportAgentWithVolumes;
use serde_json::json;
use sqlx::{Postgres, Transaction};
use volume_domain::VolumeMountScope;

use crate::release_volume_instance_inputs::{
    ImportInputs, PublishedVolumeAgent, exact_replay, record_exact, resolve_inputs,
};
use crate::{
    ReleaseService, ReleaseServiceError, append_event, append_instance_event, insert_revision,
};

impl ReleaseService {
    /// Imports exact published declarations, bindings and grants atomically.
    ///
    /// Named dispatch remains closed independently of revision completeness.
    /// No legacy origin, scalar volume pointer, or attachment is fabricated.
    ///
    /// # Errors
    ///
    /// Rejects live authorization loss before replay, mismatched pins, incomplete
    /// contracts, unsupported secrets/capabilities, changed input, or storage failure.
    pub async fn import_agent_with_volumes(
        &self,
        identity: &AuthenticatedIdentity,
        mut command: ImportAgentWithVolumes,
    ) -> Result<AgentInstanceId, ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        authorize_import(self, &mut tx, identity, &command).await?;
        let release: PublishedVolumeAgent = sqlx::query_as(
            "SELECT agent.family_id,agent.parameter_schema,agent.secret_slot_schema,
             agent.runtime_contract,agent.runtime_contract_hash,agent.requires_state,agent.publication_mode
             FROM release_agents agent JOIN releases release ON release.id=agent.release_id
             WHERE agent.id=$1 AND release.id=$2 AND release.state='published'",
        ).bind(command.release_agent_id.as_uuid()).bind(command.release_id.as_uuid())
            .fetch_optional(&mut *tx).await?.ok_or(ReleaseServiceError::Unavailable)?;
        let inputs = resolve_inputs(&mut tx, identity, &mut command, &release).await?;
        if let Some((id, _)) = exact_replay(
            &mut tx,
            command.command_key,
            "import_agent_with_volumes",
            &inputs.input_hash,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(AgentInstanceId::from_uuid(id));
        }
        sqlx::query("INSERT INTO agent_instances(id,project_id,family_id,name,state,active_revision_id,state_volume_id,created_by,volume_mode,run_gate_open) VALUES($1,$2,$3,$4,'active',NULL,NULL,$5,'named',false)")
            .bind(command.instance_id.as_uuid()).bind(command.project_id.as_uuid()).bind(release.family_id).bind(command.name.as_str()).bind(identity.user_id.as_uuid()).execute(&mut *tx).await?;
        insert_revision(
            &mut tx,
            command.revision_id,
            command.instance_id,
            command.release_agent_id,
            &inputs.parameters,
            &command.selected_policy,
            &inputs.effective_policy,
            &command.platform_policy_version,
            &release.publication_mode,
            None,
            true,
            &[],
            identity,
        )
        .await?;
        insert_bindings(self, &mut tx, identity, &command, &release, &inputs).await?;
        sqlx::query("UPDATE agent_instances SET active_revision_id=$2,updated_at=now(),version=version+1 WHERE id=$1 AND active_revision_id IS NULL")
            .bind(command.instance_id.as_uuid()).bind(command.revision_id.as_uuid()).execute(&mut *tx).await?;
        record_exact(
            &mut tx,
            command.command_key,
            "import_agent_with_volumes",
            command.instance_id.as_uuid(),
            Some(command.revision_id.as_uuid()),
            identity,
            &inputs.input_hash,
        )
        .await?;
        append_instance_event(
            &mut tx,
            command.instance_id,
            Some(command.revision_id),
            "instance.created",
            identity,
            json!({"runnable":true,"volume_mode":"named","dispatch_supported":false}),
        )
        .await?;
        append_event(&mut tx,command.instance_id.as_uuid(),"hephaestus.agent_instance.created.v1","agent_instance.created.v1",json!({"instance_id":command.instance_id,"revision_id":command.revision_id,"release_agent_id":command.release_agent_id,"project_id":command.project_id,"runnable":true,"volume_mode":"named","dispatch_supported":false})).await?;
        tx.commit().await?;
        Ok(command.instance_id)
    }
}

async fn authorize_import(
    service: &ReleaseService,
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    command: &ImportAgentWithVolumes,
) -> Result<(), ReleaseServiceError> {
    service
        .require(
            tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::Project, command.project_id.as_uuid()),
        )
        .await?;
    service
        .require(
            tx,
            identity,
            Permission::CanUse,
            ObjectRef::new(ObjectType::ReleaseAgent, command.release_agent_id.as_uuid()),
        )
        .await?;
    if command.volumes.len() > volume_domain::MAX_VOLUME_SLOTS {
        return Err(ReleaseServiceError::InvalidVolumeSelection);
    }
    for volume in &command.volumes {
        for permission in [Permission::CanAttach, Permission::CanGrantAgentCapability] {
            service
                .require(
                    tx,
                    identity,
                    permission,
                    ObjectRef::new(ObjectType::StateVolume, volume.volume_id.as_uuid()),
                )
                .await?;
        }
    }
    Ok(())
}

async fn insert_bindings(
    service: &ReleaseService,
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    command: &ImportAgentWithVolumes,
    release: &PublishedVolumeAgent,
    inputs: &ImportInputs,
) -> Result<(), ReleaseServiceError> {
    for selection in &command.volumes {
        let declaration = inputs
            .declarations
            .iter()
            .find(|slot| slot.slot() == &selection.slot)
            .ok_or(ReleaseServiceError::InvalidVolumeSelection)?;
        let provenance = if release.requires_state && selection.slot.as_str() == "state" {
            "legacy_declaration"
        } else {
            "explicit"
        };
        sqlx::query("INSERT INTO agent_instance_revision_volume_bindings(instance_revision_id,instance_id,project_id,release_agent_id,slot_key,volume_id,access_mode,guest_path,slot_required,minimum_capacity_bytes,provenance,created_by) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)")
            .bind(command.revision_id.as_uuid()).bind(command.instance_id.as_uuid()).bind(command.project_id.as_uuid()).bind(command.release_agent_id.as_uuid()).bind(selection.slot.as_str()).bind(selection.volume_id.as_uuid()).bind(selection.access_mode.as_str()).bind(selection.guest_path.as_str()).bind(declaration.required()).bind(i64::try_from(declaration.minimum_capacity_bytes()).map_err(|_|ReleaseServiceError::InvalidVolumeSelection)?).bind(provenance).bind(identity.user_id.as_uuid()).execute(&mut **tx).await?;
        let scope = VolumeMountScope::new(
            command.instance_id,
            command.revision_id,
            command.release_agent_id,
            selection.slot.clone(),
            selection.volume_id,
            selection.access_mode,
            inputs.contract_hash,
        )
        .map_err(|_| ReleaseServiceError::InvalidVolumeSelection)?;
        crate::release_volume_grants::grant_in_transaction(
            service,
            tx,
            identity,
            selection.grant_id,
            &scope,
        )
        .await?;
    }
    Ok(())
}
