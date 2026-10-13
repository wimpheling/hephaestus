//! Worker DTO fixture facts test SQL semantics only, never host custody/readiness.
pub use crate::backing::{begin, observation, revoke};
pub use crate::seals::{Fixture, identity};
use sqlx::PgPool;
use uuid::Uuid;
use volume_trait::{
    OwnedPartialBirthObservation, OwnedPartialBirthObservationWire, OwnedPartialBirthPhase,
    OwnedPartialRetentionContext,
};
pub async fn setup(pool: &PgPool) -> Fixture {
    crate::seals::checkpoint(112)
        .run(pool)
        .await
        .expect("private112 checkpoint with111foundation");
    crate::seals::seed(pool).await
}
pub fn partial(
    context: &OwnedPartialRetentionContext,
    phase: OwnedPartialBirthPhase,
) -> OwnedPartialBirthObservation {
    let purpose = &context.claim.purpose;
    let receipt = purpose.receipt();
    let resource = receipt.intent.registration();
    OwnedPartialBirthObservation::try_from(OwnedPartialBirthObservationWire {
        id: Uuid::new_v4(),
        observation_version: context.head.version + 1,
        volume_id: resource.id().as_uuid(),
        project_id: resource.project_id(),
        seal_id: receipt.seal_id,
        original_creation: receipt.intent.creation().fields().clone(),
        provisioning_operation_id: context.claim.operation_id,
        purpose_hash: purpose.hash(),
        generation: context.claim.generation,
        phase,
        declared_capacity_bytes: resource.capacity_bytes(),
        actual_length_bytes: if phase == OwnedPartialBirthPhase::ClaimedEmpty {
            0
        } else {
            resource.capacity_bytes()
        },
        owner_namespace: purpose.owner_namespace(),
        root_device: 2,
        root_inode: 10,
        root_owner_uid: 1000,
        namespace_device: 2,
        namespace_inode: 11,
        owner_uid: 1000,
        backing_device: 2,
        backing_inode: 12,
        birth_record_hash: [1; 32],
        inode_record_hash: [2; 32],
        journal_hash: [3; 32],
    })
    .unwrap()
}
