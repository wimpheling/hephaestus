use super::{
    fixture::Fixture,
    instance::{Prepared, key},
};
use hephaestus_app::recipe_owned_volume::PreparedOwnedVolumeCreator;
use recipe_application::InstallProgress;
use runtime_types::VolumeId;
use volume_trait::{
    OwnedBackingPhase, OwnedProvisioningDiscovery, OwnedProvisioningExpectation,
    OwnedVolumeRegistration, VolumeCreationIdentity, VolumeCreationIdentityWire,
    VolumeOwnedProvisioningRepository, VolumeProvisioningState, VolumeRegistration,
    VolumeResourceRepository, VolumeState,
};

pub async fn create(fixture: &Fixture, prepared: &Prepared) -> VolumeId {
    let creator = PreparedOwnedVolumeCreator::new(
        prepared.preparation.clone(),
        prepared.repository.clone(),
        fixture.actor_metadata.clone(),
        fixture.metadata.clone(),
        prepared.recorder.clone(),
        fixture.local_config.clone(),
        fixture.owner.clone(),
    )
    .unwrap();
    let progress = creator
        .create_prepared_volume(
            &prepared.actor,
            prepared.command,
            prepared.deployment,
            &key("data"),
        )
        .await
        .unwrap();
    assert_eq!(progress.install, InstallProgress::Ready);
    assert!(progress.active_attempt.is_none());
    let volume = prepared.instance.volumes()[0].volume_id();
    let inspection = fixture
        .actor_metadata
        .inspect(&prepared.actor, volume)
        .await
        .unwrap();
    assert_eq!(inspection.capacity_bytes, 16_777_216);
    assert_eq!(inspection.lifecycle_state, VolumeState::Ready);
    assert_eq!(
        inspection.provisioning_state,
        VolumeProvisioningState::Ready
    );
    let actual: (uuid::Uuid, i64, uuid::Uuid, serde_json::Value) = sqlx::query_as(
        "SELECT project_id,capacity_bytes,filesystem_uuid,creation_json FROM recipe_owned_volume_preparations WHERE volume_id=$1",
    ).bind(volume.as_uuid()).fetch_one(&fixture.admin).await.unwrap();
    let creation = VolumeCreationIdentity::try_from(
        serde_json::from_value::<VolumeCreationIdentityWire>(actual.3).unwrap(),
    )
    .unwrap();
    let registration = OwnedVolumeRegistration::new(
        VolumeRegistration::new(volume, actual.0, u64::try_from(actual.1).unwrap(), actual.2)
            .unwrap(),
        creation,
    );
    let expected = OwnedProvisioningExpectation::new(
        registration,
        fixture.owner.host().into(),
        fixture.owner.root().into(),
        fixture.owner.namespace_id(),
    )
    .unwrap();
    let OwnedProvisioningDiscovery::Original(original) = fixture
        .metadata
        .discover_owned_provisioning(&expected)
        .await
        .unwrap()
    else {
        panic!("completed actual first Ready must have its genuine original")
    };
    assert_eq!(original.claim.generation, 1);
    let observed = original.observation.as_ref().unwrap().fields();
    assert_eq!(observed.phase, OwnedBackingPhase::Ready);
    assert_eq!(observed.capacity_bytes, 16_777_216);
    assert_eq!(
        observed.filesystem.as_ref().unwrap().filesystem_uuid,
        actual.2
    );
    assert!(observed.filesystem.as_ref().unwrap().clean);
    fixture.owner.validate().unwrap();
    volume
}
