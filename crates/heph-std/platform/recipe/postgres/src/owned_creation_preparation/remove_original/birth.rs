//! Actual sealed 110/111 lineage; no filesystem check or birth adoption.
use super::super::mapping;
use crate::repository_error;
use recipe_application::{DeploymentError, EffectClaim, PreparedOwnedVolumeCreation};
use sqlx::{Postgres, Transaction};

pub(super) async fn qualify(
    tx: &mut Transaction<'_, Postgres>,
    claim: &EffectClaim,
    prepared: &PreparedOwnedVolumeCreation,
) -> Result<(), DeploymentError> {
    let rows: Vec<Vec<u8>> = sqlx::query_scalar("SELECT stored.canonical_claim
        FROM recipe_owned_volume_preparations stored
        JOIN volume_creation_seals seal ON seal.volume_id=stored.volume_id
        JOIN agent_instance_state_volumes volume ON volume.id=seal.volume_id
        JOIN volume_owned_backing_purposes purpose ON purpose.volume_id=volume.id AND purpose.seal_id=seal.id
        JOIN volume_owned_provisioning_operations operation ON operation.volume_id=volume.id
        JOIN volume_owned_backing_observations ready ON ready.volume_id=volume.id AND ready.operation_id=operation.id
        WHERE stored.attempt_id=$1 AND stored.deployment_id=$2 AND stored.resource_name=$3
          AND stored.canonical_claim=$4 AND stored.claim_digest=sha256(stored.canonical_claim)
          AND stored.creation_json=$5 AND stored.actor_id=$7 AND stored.request_id<>'00000000-0000-0000-0000-000000000000'::uuid
          AND EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox ON outbox.event_id=event.id
            WHERE event.id=stored.event_id AND event.occurrence_id=$14
            AND event.scope_kind='project' AND event.scope_id=stored.project_id
            AND event.aggregate_type='project' AND event.aggregate_id=stored.project_id AND event.event_type='project.changed'
            AND event.change_kind='updated' AND event.schema_version=1 AND event.actor_type='user' AND event.actor_id=stored.actor_id
            AND event.request_id=stored.request_id AND event.cursor=stored.event_cursor AND event.aggregate_version=stored.event_aggregate_version)
          AND stored.scope_id=seal.scope_id AND stored.effect_id=seal.effect_id
          AND stored.registration_operation_id=seal.operation_id
          AND stored.volume_id=seal.volume_id AND stored.project_id=seal.project_id
          AND stored.capacity_bytes=seal.capacity_bytes AND stored.filesystem_uuid=seal.filesystem_uuid
          AND seal.attempt_id=$1 AND seal.command_id=$6 AND seal.actor_id=$7 AND seal.origin_request_id=$8
          AND seal.resource_version=$9 AND seal.generation=$10 AND seal.input_hash=$11
          AND seal.intent_hash=$12 AND seal.registration_hash=sha256(seal.canonical_bytes)
          AND volume.creation_seal_id=seal.id AND volume.creation_operation_id=seal.operation_id
          AND volume.project_id=seal.project_id AND volume.capacity_bytes=seal.capacity_bytes
          AND volume.filesystem_uuid=seal.filesystem_uuid
          AND volume.provisioning_state='ready' AND volume.provisioning_generation=1
          AND volume.state IN ('ready','attached','recovering')
          AND purpose.project_id=seal.project_id AND purpose.birth_generation=1
          AND purpose.purpose_hash=sha256(purpose.canonical_bytes)
          AND volume.host_id=purpose.host_id AND volume.host_path=purpose.canonical_path
          AND purpose.owner_namespace<>'00000000-0000-0000-0000-000000000000'::uuid
          AND operation.id=$13 AND operation.expected_generation=0 AND operation.generation=1
          AND operation.actor_id=seal.actor_id AND operation.purpose_hash=purpose.purpose_hash
          AND ready.generation=1 AND (SELECT count(*) FROM jsonb_object_keys(ready.proof_json))=17
          AND (SELECT count(*) FROM jsonb_object_keys(ready.proof_json->'filesystem'))=4
          AND (ready.proof_json->'filesystem'->>'block_size')::bigint=4096
          AND ready.phase='ready'
          AND ready.evidence_hash=sha256(convert_to(ready.proof_json::text,'UTF8'))
          AND ready.proof_json->>'id'=ready.id::text AND ready.proof_json->>'phase'='ready'
          AND (ready.proof_json->>'generation')::bigint=1
          AND ready.proof_json->'purpose_hash'=to_jsonb(ARRAY(SELECT get_byte(purpose.purpose_hash,n) FROM generate_series(0,31) n))
          AND (ready.proof_json->>'capacity_bytes')::bigint=seal.capacity_bytes
          AND (ready.proof_json->'filesystem'->>'filesystem_uuid')::uuid=seal.filesystem_uuid
          AND (ready.proof_json->'filesystem'->>'clean')::boolean IS TRUE
          AND (ready.proof_json->'filesystem'->>'block_size')::numeric*(ready.proof_json->'filesystem'->>'block_count')::numeric=seal.capacity_bytes
          AND NOT EXISTS(SELECT 1 FROM volume_owned_birth_retirements WHERE volume_id=volume.id)
          AND (SELECT count(*) FROM volume_owned_provisioning_operations WHERE volume_id=volume.id)=1
          AND EXISTS(SELECT 1 FROM volume_owned_transition_proofs proof WHERE proof.id=operation.id
            AND proof.volume_id=volume.id AND proof.operation_id=operation.id AND proof.mode='claim'
            AND proof.previous_generation=0 AND proof.generation=1)
          AND EXISTS(SELECT 1 FROM volume_owned_transition_proofs proof WHERE proof.id=ready.id
            AND proof.volume_id=volume.id AND proof.operation_id=operation.id AND proof.mode='ready'
            AND proof.previous_generation=1 AND proof.generation=1)
          AND EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox ON outbox.event_id=event.id
            WHERE event.id=seal.event_id AND event.occurrence_id=seal.operation_id
            AND event.scope_kind='project' AND event.scope_id=seal.project_id
            AND event.aggregate_type='project' AND event.aggregate_id=seal.project_id AND event.event_type='project.changed'
            AND event.change_kind='created' AND event.schema_version=1 AND event.actor_type='user' AND event.actor_id=seal.actor_id
            AND event.request_id=seal.first_request_id AND event.cursor=seal.event_cursor AND event.aggregate_version=seal.event_aggregate_version)
          AND EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox ON outbox.event_id=event.id
            WHERE event.id=operation.event_id AND event.occurrence_id=operation.id
            AND event.scope_kind='project' AND event.scope_id=seal.project_id
            AND event.aggregate_type='project' AND event.aggregate_id=seal.project_id AND event.event_type='project.changed'
            AND event.change_kind='updated' AND event.actor_type='user' AND event.actor_id=seal.actor_id AND event.request_id=operation.request_id)
          AND EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox ON outbox.event_id=event.id
            WHERE event.id=ready.event_id AND event.occurrence_id=ready.id
            AND event.scope_kind='project' AND event.scope_id=seal.project_id
            AND event.aggregate_type='project' AND event.aggregate_id=seal.project_id AND event.event_type='project.changed'
            AND event.change_kind='updated' AND event.schema_version=1
            AND event.actor_type='system' AND event.actor_id IS NULL)")
        .bind(claim.provenance.attempt_id.as_uuid()).bind(claim.deployment_id.as_uuid()).bind(claim.resource.as_str())
        .bind(mapping::canonical_claim(claim)?)
        .bind(serde_json::to_value(prepared.creation.fields()).map_err(|_| DeploymentError::Serialization)?)
        .bind(claim.command.id().as_uuid()).bind(claim.provenance.actor_id.as_uuid()).bind(claim.provenance.request_id.as_uuid())
        .bind(i64::try_from(claim.resource_version).map_err(|_| DeploymentError::ReconciliationRequired)?)
        .bind(i64::try_from(claim.generation).map_err(|_| DeploymentError::ReconciliationRequired)?)
        .bind(claim.input_hash.as_bytes().as_slice()).bind(prepared.creation.fields().intent_hash.as_slice())
        .bind(uuid::Uuid::new_v5(&prepared.creation.fields().operation_id.as_uuid(), b"heph-owned-first-provision-v1\0"))
        .bind(uuid::Uuid::new_v5(&claim.provenance.attempt_id.as_uuid(), b"hephaestus-recipe-owned-volume-preparation-v1\0"))
        .fetch_all(&mut **tx).await.map_err(repository_error)?;
    canonical(&rows, &mapping::canonical_claim(claim)?)
}

pub(super) fn canonical(rows: &[Vec<u8>], expected: &[u8]) -> Result<(), DeploymentError> {
    match rows {
        [actual] if actual.as_slice() == expected => Ok(()),
        _ => Err(DeploymentError::ReconciliationRequired),
    }
}
