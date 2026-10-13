use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use volume_trait::{
    OwnedBackingObservation, OwnedBackingPhase, OwnedBackingPurpose, OwnedProvisioningExpectation,
    VolumeError,
};

use super::super::storage::OperationRow;
use crate::errors::metadata;

#[derive(sqlx::FromRow)]
pub(super) struct VolumeRow {
    host_id: Option<String>,
    host_path: Option<String>,
    provisioning_generation: i64,
    provisioning_state: String,
    state: String,
}
impl VolumeRow {
    pub(super) fn admitted(
        &self,
        purpose: &OwnedBackingPurpose,
        observation: Option<&OwnedBackingObservation>,
    ) -> Result<(), VolumeError> {
        let expected_state = match observation.map(|value| value.fields().phase) {
            None => "reserved",
            Some(OwnedBackingPhase::Recorded) => "creating",
            Some(OwnedBackingPhase::FormatIntent) => "formatting",
            Some(OwnedBackingPhase::Ready) => "ready",
            Some(OwnedBackingPhase::FailedBeforeFormat | OwnedBackingPhase::Uncertain) => {
                "uncertain"
            }
        };
        let canonical = purpose.canonical_path();
        if self.provisioning_generation != 1
            || self.host_id.as_deref() != Some(purpose.host())
            || self.host_path.as_deref() != canonical.to_str()
            || self.provisioning_state != expected_state
            || (expected_state != "ready" && self.state != "uninitialized")
            || (expected_state == "ready"
                && !matches!(self.state.as_str(), "ready" | "attached" | "recovering"))
        {
            return Err(VolumeError::ProvisioningUncertain(
                "current metadata differs from original operation",
            ));
        }
        Ok(())
    }
}
pub(super) async fn volume(
    tx: &mut Transaction<'_, Postgres>,
    purpose: &OwnedBackingPurpose,
) -> Result<VolumeRow, VolumeError> {
    let receipt = purpose.receipt();
    sqlx::query_as("SELECT host_id,host_path,provisioning_generation,provisioning_state,state FROM agent_instance_state_volumes WHERE id=$1 AND instance_id IS NULL AND project_id=$2 AND creation_seal_id=$3 AND creation_operation_id=$4 AND capacity_bytes=$5 AND filesystem_uuid=$6")
        .bind(receipt.intent.registration().id().as_uuid()).bind(receipt.intent.registration().project_id())
        .bind(receipt.seal_id.as_uuid()).bind(receipt.intent.creation().fields().operation_id.as_uuid())
        .bind(i64::try_from(receipt.intent.registration().capacity_bytes()).map_err(metadata)?)
        .bind(receipt.intent.registration().filesystem_uuid()).fetch_optional(&mut **tx).await.map_err(metadata)?
        .ok_or(VolumeError::IntentConflict)
}
pub(super) async fn operations(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> Result<Vec<OperationRow>, VolumeError> {
    sqlx::query_as("SELECT * FROM volume_owned_provisioning_operations WHERE volume_id=$1 ORDER BY generation,id")
        .bind(id).fetch_all(&mut **tx).await.map_err(metadata)
}
pub(super) async fn unadmitted(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    row: &VolumeRow,
) -> Result<(), VolumeError> {
    let zero: bool = sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM volume_owned_backing_purposes WHERE volume_id=$1) AND NOT EXISTS(SELECT 1 FROM volume_owned_backing_observations WHERE volume_id=$1) AND NOT EXISTS(SELECT 1 FROM volume_owned_transition_proofs WHERE volume_id=$1) AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE volume_id=$1)")
        .bind(id).fetch_one(&mut **tx).await.map_err(metadata)?;
    if !zero
        || row.provisioning_generation != 0
        || row.provisioning_state != "reserved"
        || row.state != "uninitialized"
        || row.host_id.is_some()
        || row.host_path.is_some()
    {
        return Err(VolumeError::ProvisioningUncertain(
            "unadmitted birth has nonzero or orphan history",
        ));
    }
    Ok(())
}
pub(super) async fn purpose(
    tx: &mut Transaction<'_, Postgres>,
    purpose: &OwnedBackingPurpose,
) -> Result<(), VolumeError> {
    let receipt = purpose.receipt();
    let exact: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM volume_owned_backing_purposes WHERE volume_id=$1 AND seal_id=$2 AND project_id=$3 AND host_id=$4 AND root_path=$5 AND owner_namespace=$6 AND namespace_name=$7 AND canonical_path=$8 AND birth_generation=1 AND canonical_bytes=$9 AND purpose_hash=$10)")
        .bind(receipt.intent.registration().id().as_uuid()).bind(receipt.seal_id.as_uuid())
        .bind(receipt.intent.registration().project_id()).bind(purpose.host()).bind(purpose.root().to_str())
        .bind(purpose.owner_namespace().as_uuid()).bind(purpose.namespace_name()).bind(purpose.canonical_path().to_str())
        .bind(purpose.canonical_bytes()).bind(purpose.hash().as_slice()).fetch_one(&mut **tx).await.map_err(metadata)?;
    if exact {
        Ok(())
    } else {
        Err(VolumeError::IntentConflict)
    }
}
pub(super) async fn operation(
    tx: &mut Transaction<'_, Postgres>,
    expected: &OwnedProvisioningExpectation,
    purpose: &OwnedBackingPurpose,
    operation: &OperationRow,
) -> Result<(), VolumeError> {
    if operation.id != expected.first_operation_id()
        || operation.expected_generation != 0
        || operation.generation != 1
        || operation.request_id.is_nil()
        || operation.event_id.is_nil()
    {
        return Err(VolumeError::IntentConflict);
    }
    let exact: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM application_events event WHERE event.id=$1 AND event.occurrence_id=$2 AND event.scope_kind='project' AND event.scope_id=$3 AND event.aggregate_type='project' AND event.aggregate_id=$3 AND event.event_type='project.changed' AND event.change_kind='updated' AND event.actor_id=$4 AND event.request_id=$5) AND EXISTS(SELECT 1 FROM volume_owned_transition_proofs WHERE volume_id=$6 AND id=$2 AND operation_id=$2 AND mode='claim' AND generation=1 AND previous_generation=0) AND NOT EXISTS(SELECT 1 FROM volume_owned_transition_proofs proof WHERE proof.volume_id=$6 AND (proof.operation_id<>$2 OR proof.generation<>1 OR (proof.mode='claim' AND (proof.id<>$2 OR proof.previous_generation<>0)) OR (proof.mode<>'claim' AND (proof.previous_generation<>1 OR NOT EXISTS(SELECT 1 FROM volume_owned_backing_observations observation WHERE observation.id=proof.id AND observation.volume_id=$6 AND observation.operation_id=$2 AND observation.generation=1 AND observation.phase=proof.mode)))))")
        .bind(operation.event_id).bind(operation.id).bind(purpose.receipt().intent.registration().project_id())
        .bind(operation.actor_id).bind(operation.request_id).bind(operation.volume_id).fetch_one(&mut **tx).await.map_err(metadata)?;
    if exact {
        Ok(())
    } else {
        Err(VolumeError::IntentConflict)
    }
}

pub(super) async fn birth_event(
    tx: &mut Transaction<'_, Postgres>,
    purpose: &OwnedBackingPurpose,
) -> Result<(), VolumeError> {
    let receipt = purpose.receipt();
    let exact: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM application_events event WHERE event.id=$1 AND event.occurrence_id=$2 AND event.scope_kind='project' AND event.scope_id=$3 AND event.aggregate_type='project' AND event.aggregate_id=$3 AND event.event_type='project.changed' AND event.change_kind='created' AND event.actor_id=$4 AND event.request_id=$5 AND event.cursor=$6 AND event.aggregate_version=$7)")
        .bind(receipt.event_id).bind(receipt.intent.creation().fields().operation_id.as_uuid())
        .bind(receipt.intent.registration().project_id()).bind(receipt.intent.creation().fields().actor_id)
        .bind(receipt.first_request_id).bind(receipt.event_cursor).bind(receipt.event_aggregate_version)
        .fetch_one(&mut **tx).await.map_err(metadata)?;
    if exact {
        Ok(())
    } else {
        Err(VolumeError::IntentConflict)
    }
}
