use super::{birth, fixture::Core};
use hephaestus_app::recipe::ready_retention::ReadyRetentionCoordinator;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId,
    DeploymentExecutionPreparationRepository, DeploymentOperation, EffectClaim, InstallDeployment,
    OwnedReadyRetentionRepository, RemoveDeployment, ResourceAction, ResourceEffect,
    RetainOwnedReady,
};
use uuid::Uuid;
use volume_trait::{
    OwnedProvisioningContext, OwnedRetainedReuseContext, VolumeRetainedReuseRepository,
};

pub async fn retain(
    core: &Core,
    target: &EffectClaim,
    original: &OwnedProvisioningContext,
) -> OwnedRetainedReuseContext {
    let identity = crate::support::identity(core.identity.user_id);
    let snapshot = core
        .repository
        .inspect(&identity, target.deployment_id)
        .await
        .unwrap();
    let command = CommandIdentity::from_identity(&identity, DeploymentOperation::Remove).unwrap();
    let admitted = core
        .repository
        .admit_remove(
            &identity,
            RemoveDeployment {
                command,
                deployment_id: target.deployment_id,
                expected_version: snapshot.version,
            },
        )
        .await
        .unwrap();
    let request = RetainOwnedReady {
        command,
        target: target.clone(),
        operation_id: DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
        expected_deployment_version: admitted.snapshot.version,
        expected_resource_version: admitted.snapshot.resources[&birth::key()].version,
    };
    let coordinator = ReadyRetentionCoordinator::new(
        core.actor.clone(),
        core.retention_worker.clone(),
        core.native.local.clone(),
        core.native.provider.clone(),
    );
    let progress = coordinator
        .retain(&identity, &request, &original.claim)
        .await
        .unwrap();
    assert_eq!(progress.install, recipe_application::InstallProgress::Ready);
    assert_eq!(
        progress.removal,
        recipe_application::RemovalProgress::Retained
    );
    assert!(progress.active_attempt.is_none());
    let plan = core
        .actor
        .begin_owned_ready_retention(&identity, &request)
        .await
        .unwrap();
    assert_eq!(plan.completed, Some(progress));
    core.retention_worker
        .retained_reuse_context(
            &original.claim,
            plan.closure_id,
            request.operation_id.as_uuid(),
        )
        .await
        .unwrap()
}

pub async fn external(
    core: &Core,
    original: &OwnedProvisioningContext,
) -> (AuthenticatedIdentity, EffectClaim) {
    let identity = crate::support::identity(core.identity.user_id);
    let volume = original.claim.purpose.receipt().intent.registration().id();
    let intent = birth::plan(core, &identity, Some(volume)).await;
    let command = CommandIdentity::from_identity(&identity, DeploymentOperation::Install).unwrap();
    core.repository
        .admit_install(
            &identity,
            InstallDeployment {
                command,
                intent: intent.clone(),
            },
        )
        .await
        .unwrap();
    core.preparation
        .prepare_execution(&identity, command, intent.id())
        .await
        .unwrap();
    let snapshot = core
        .repository
        .inspect(&identity, intent.id())
        .await
        .unwrap();
    let claim = core
        .repository
        .claim_resource_effect(
            &identity,
            ResourceEffect {
                command,
                deployment_id: intent.id(),
                resource: birth::key(),
                expected_deployment_version: snapshot.version,
                expected_resource_version: snapshot.resources[&birth::key()].version,
                action: ResourceAction::VerifyExternal,
                provenance: AttemptProvenance::new(
                    &identity,
                    DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
                )
                .unwrap(),
            },
        )
        .await
        .unwrap();
    (identity, claim)
}
