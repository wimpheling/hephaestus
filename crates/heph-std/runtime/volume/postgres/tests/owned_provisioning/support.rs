use std::path::PathBuf;

use sqlx::PgPool;
use uuid::Uuid;
use volume_trait::{
    BeginOwnedProvisioning, OwnedBackingObservation, OwnedBackingObservationWire,
    OwnedBackingPhase, OwnedBackingPurpose, OwnedFilesystemBirth, OwnedProvisioningClaim,
    VolumeOwnedProvisioningRepository, VolumeOwnedRegistrationRepository, VolumeRootNamespaceId,
};

pub use crate::seals::{Fixture, identity, revoke};

pub async fn setup(pool: &PgPool) -> Fixture {
    crate::seals::checkpoint(111)
        .run(pool)
        .await
        .expect("published 111 checkpoint");
    crate::seals::seed(pool).await
}
pub async fn request(fixture: &Fixture) -> BeginOwnedProvisioning {
    let intent = crate::seals::intent(fixture);
    let receipt = fixture
        .actor
        .register_owned(&identity(fixture.owner), &intent)
        .await
        .expect("original seal");
    BeginOwnedProvisioning {
        purpose: OwnedBackingPurpose::new(
            receipt,
            String::from("owned-host"),
            PathBuf::from("/owned-volumes"),
            VolumeRootNamespaceId::from_uuid(Uuid::new_v4()).unwrap(),
            1,
        )
        .unwrap(),
        operation_id: Uuid::new_v4(),
        expected_generation: 0,
    }
}
pub async fn begin(fixture: &Fixture) -> OwnedProvisioningClaim {
    fixture
        .actor
        .begin_owned_provisioning(&identity(fixture.owner), &request(fixture).await)
        .await
        .unwrap()
        .claim
}
pub fn observation(
    claim: &OwnedProvisioningClaim,
    phase: OwnedBackingPhase,
) -> OwnedBackingObservation {
    OwnedBackingObservation::try_from(OwnedBackingObservationWire {
        id: Uuid::new_v4(),
        purpose_hash: claim.purpose.hash(),
        generation: claim.generation,
        phase,
        root_device: 2,
        root_inode: 10,
        namespace_device: 2,
        namespace_inode: 11,
        owner_uid: 1000,
        backing_device: 2,
        backing_inode: 12,
        birth_record_hash: [1; 32],
        inode_record_hash: [2; 32],
        journal_hash: [3; 32],
        never_format_started: matches!(
            phase,
            OwnedBackingPhase::Recorded | OwnedBackingPhase::FailedBeforeFormat
        ),
        capacity_bytes: claim
            .purpose
            .receipt()
            .intent
            .registration()
            .capacity_bytes(),
        filesystem: (phase == OwnedBackingPhase::Ready).then(|| OwnedFilesystemBirth {
            filesystem_uuid: claim
                .purpose
                .receipt()
                .intent
                .registration()
                .filesystem_uuid(),
            block_size: 4096,
            block_count: claim
                .purpose
                .receipt()
                .intent
                .registration()
                .capacity_bytes()
                / 4096,
            clean: true,
        }),
    })
    .unwrap()
}
