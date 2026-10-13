use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use volume_trait::{
    OwnedBackingObservation, OwnedBackingObservationWire, OwnedProvisioningClaim, VolumeError,
};

use crate::errors::metadata;

#[derive(sqlx::FromRow)]
struct ObservationRow {
    id: Uuid,
    birth_observation_id: Uuid,
    proof_json: serde_json::Value,
    exact: bool,
}

pub(super) async fn load(
    tx: &mut Transaction<'_, Postgres>,
    claim: &OwnedProvisioningClaim,
) -> Result<Option<OwnedBackingObservation>, VolumeError> {
    // LEFT JOIN retains orphan history rather than filtering it into absence.
    let rows: Vec<ObservationRow> = sqlx::query_as("SELECT observation.id,observation.birth_observation_id,observation.proof_json,COALESCE(observation.operation_id=$2 AND observation.generation=1 AND observation.evidence_hash=sha256(convert_to(observation.proof_json::text,'UTF8')) AND observation.phase=observation.proof_json->>'phase' AND event.occurrence_id=observation.id AND event.scope_kind='project' AND event.scope_id=$3 AND event.aggregate_type='project' AND event.aggregate_id=$3 AND event.event_type='project.changed' AND EXISTS(SELECT 1 FROM volume_owned_transition_proofs transition WHERE transition.id=observation.id AND transition.volume_id=$1 AND transition.operation_id=$2 AND transition.generation=1 AND transition.previous_generation=1 AND transition.mode=observation.phase),false) AS exact FROM volume_owned_backing_observations observation LEFT JOIN application_events event ON event.id=observation.event_id WHERE observation.volume_id=$1 ORDER BY event.cursor NULLS FIRST,observation.id")
        .bind(claim.purpose.receipt().intent.registration().id().as_uuid()).bind(claim.operation_id)
        .bind(claim.purpose.receipt().intent.registration().project_id()).fetch_all(&mut **tx).await.map_err(metadata)?;
    let mut first: Option<OwnedBackingObservation> = None;
    let mut latest = None;
    for row in rows {
        let observation = OwnedBackingObservation::try_from(
            serde_json::from_value::<OwnedBackingObservationWire>(row.proof_json)
                .map_err(metadata)?,
        )?;
        validate(claim, &observation, row.id, row.exact)?;
        if let Some(original) = &first {
            if row.birth_observation_id != original.fields().id
                || !same_birth(original, &observation)
            {
                return Err(VolumeError::IntentConflict);
            }
        } else {
            if row.birth_observation_id != row.id {
                return Err(VolumeError::IntentConflict);
            }
            first = Some(observation.clone());
        }
        latest = Some(observation);
    }
    Ok(latest)
}

fn validate(
    claim: &OwnedProvisioningClaim,
    observation: &OwnedBackingObservation,
    id: Uuid,
    exact: bool,
) -> Result<(), VolumeError> {
    let fields = observation.fields();
    let registration = claim.purpose.receipt().intent.registration();
    if !exact
        || id != fields.id
        || fields.generation != claim.generation
        || fields.purpose_hash != claim.purpose.hash()
        || fields.capacity_bytes != registration.capacity_bytes()
        || fields
            .filesystem
            .as_ref()
            .is_some_and(|filesystem| filesystem.filesystem_uuid != registration.filesystem_uuid())
    {
        return Err(VolumeError::IntentConflict);
    }
    Ok(())
}
fn same_birth(first: &OwnedBackingObservation, next: &OwnedBackingObservation) -> bool {
    let left = first.fields();
    let right = next.fields();
    left.root_device == right.root_device
        && left.root_inode == right.root_inode
        && left.namespace_device == right.namespace_device
        && left.namespace_inode == right.namespace_inode
        && left.owner_uid == right.owner_uid
        && left.backing_device == right.backing_device
        && left.backing_inode == right.backing_inode
        && left.birth_record_hash == right.birth_record_hash
        && left.inode_record_hash == right.inode_record_hash
}
