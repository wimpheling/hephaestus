use super::fixture::Core;
use capability_domain::CapabilitySlotKey;
use hephaestus_app::recipe_owned_volume::PreparedOwnedVolumeCreator;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId,
    DeploymentExecutionPreparationRepository, DeploymentIntent, DeploymentKey, DeploymentOperation,
    EffectClaim, InstallDeployment, PlanningRequest, RecipePlanner, ResourceAction, ResourceEffect,
};
use recipe_postgres::PostgresPlanningCatalog;
use std::collections::BTreeMap;
use uuid::Uuid;
use volume_trait::{
    OwnedProvisioningContext, OwnedProvisioningDiscovery, OwnedProvisioningExpectation,
    OwnedVolumeRegistration, VolumeOwnedProvisioningRepository, VolumeRegistration,
};

pub fn key() -> CapabilitySlotKey {
    CapabilitySlotKey::parse("data").unwrap()
}

pub async fn create(core: &Core) -> (EffectClaim, OwnedProvisioningContext) {
    let intent = plan(core, &core.identity, None).await;
    let command =
        CommandIdentity::from_identity(&core.identity, DeploymentOperation::Install).unwrap();
    core.repository
        .admit_install(
            &core.identity,
            InstallDeployment {
                command,
                intent: intent.clone(),
            },
        )
        .await
        .unwrap();
    core.preparation
        .prepare_execution(&core.identity, command, intent.id())
        .await
        .unwrap();
    let snapshot = core
        .repository
        .inspect(&core.identity, intent.id())
        .await
        .unwrap();
    let target = core
        .repository
        .claim_resource_effect(
            &core.identity,
            ResourceEffect {
                command,
                deployment_id: intent.id(),
                resource: key(),
                expected_deployment_version: snapshot.version,
                expected_resource_version: snapshot.resources[&key()].version,
                action: ResourceAction::Create,
                provenance: AttemptProvenance::new(
                    &core.identity,
                    DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
                )
                .unwrap(),
            },
        )
        .await
        .unwrap();
    let mapped = core
        .repository
        .prepare_owned_volume_creation(&core.identity, &target)
        .await
        .unwrap();
    let creator = PreparedOwnedVolumeCreator::new(
        core.preparation.clone(),
        core.repository.clone(),
        core.actor_metadata.clone(),
        core.metadata.clone(),
        core.recorder.clone(),
        core.native.config.clone(),
        core.native.owner.clone(),
    )
    .unwrap();
    let progress = creator
        .create_prepared_volume(&core.identity, command, intent.id(), &key())
        .await
        .unwrap();
    assert_eq!(progress.install, recipe_application::InstallProgress::Ready);
    assert!(progress.active_attempt.is_none());
    let registration = OwnedVolumeRegistration::new(
        VolumeRegistration::new(
            mapped.volume_id,
            mapped.project_id,
            mapped.capacity_bytes,
            mapped.filesystem_uuid,
        )
        .unwrap(),
        mapped.creation,
    );
    let expected = OwnedProvisioningExpectation::new(
        registration,
        core.native.owner.host().into(),
        core.native.owner.root().into(),
        core.native.owner.namespace_id(),
    )
    .unwrap();
    let OwnedProvisioningDiscovery::Original(original) = core
        .metadata
        .discover_owned_provisioning(&expected)
        .await
        .unwrap()
    else {
        panic!("actual completed original birth must be discoverable")
    };
    assert_eq!(original.claim.generation, 1);
    (target, *original)
}

pub async fn plan(
    core: &Core,
    identity: &AuthenticatedIdentity,
    external: Option<runtime_types::VolumeId>,
) -> DeploymentIntent {
    // Zero-instance whole preparation is a documented production shape. The
    // authoritative planner still checks current project and external rights.
    let declaration = if external.is_some() {
        "contract_version=1\nrecipe_id=\"public139-reused\"\nrecipe_version=\"1.0.0\"\n[[resources]]\nkind=\"volume\"\nname=\"data\"\nsource={type=\"external\"}\nremoval=\"retain\"\n"
    } else {
        "contract_version=1\nrecipe_id=\"public139-created\"\nrecipe_version=\"1.0.0\"\n[[resources]]\nkind=\"volume\"\nname=\"data\"\nsource={type=\"created\",capacity_bytes={source=\"literal\",value=16777216}}\nremoval=\"retain\"\n"
    };
    let external = external.map_or_else(BTreeMap::new, |volume| BTreeMap::from([(key(), volume)]));
    let request = PlanningRequest::new(
        core.project,
        DeploymentKey::parse(if external.is_empty() {
            "public139-created"
        } else {
            "public139-reused"
        })
        .unwrap(),
        declaration.as_bytes(),
        BTreeMap::new(),
        external,
    )
    .unwrap();
    RecipePlanner::new(PostgresPlanningCatalog::new(
        core.app.clone(),
        core.platform.clone(),
    ))
    .plan(identity, &request)
    .await
    .unwrap()
    .intent()
    .clone()
}
