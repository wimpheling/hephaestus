//! Existing controlled checked fixture; no native/provenance evidence.
use super::super::{fixture, instance_safety};
use crate::{
    AdmissionDisposition, AttemptProvenance, CommandIdentity, CommandReceipt, DeploymentAdmission,
    DeploymentAttemptId, DeploymentError, DeploymentLifecycle, DeploymentOperation,
    DeploymentSnapshot, EffectClaim, InstallProgress, OriginalInstanceClosure,
    PendingInstanceDrainClosure, PreparedInstanceClosureRepository, PreparedInstanceClosureRequest,
    PreparedInstanceSafetyTarget, RemovalProgress, RemoveContinuation, ResourceAction,
    ResourceProgress, ResumedPreparedInstanceClosureRequest,
};
use async_trait::async_trait;
use release_domain::ContentHash;
use std::collections::BTreeMap;
use uuid::Uuid;

pub(super) struct PendingFixture {
    pub(super) admission: DeploymentAdmission,
    pub(super) target: PreparedInstanceSafetyTarget,
    pub(super) drain: EffectClaim,
    pub(super) pending: PendingInstanceDrainClosure,
}

pub(super) fn pending_fixture() -> PendingFixture {
    let target = instance_safety::target();
    let original_actor = fixture::identity(9, 10);
    let current_actor = fixture::identity(11, 12);
    let drain = EffectClaim {
        command: CommandIdentity::from_identity(&original_actor, DeploymentOperation::Remove)
            .unwrap(),
        action: ResourceAction::Drain,
        generation: 2,
        resource_version: 3,
        provenance: AttemptProvenance::new(
            &original_actor,
            DeploymentAttemptId::from_uuid(Uuid::from_u128(50)).unwrap(),
        )
        .unwrap(),
        ..target.original().clone()
    };
    let intent = fixture::intent(fixture::SOURCE, 1);
    let current =
        CommandIdentity::from_identity(&current_actor, DeploymentOperation::Remove).unwrap();
    let snapshot = DeploymentSnapshot {
        intent: intent.clone(),
        lifecycle: DeploymentLifecycle::Removing,
        version: 9,
        resources: BTreeMap::from([(
            drain.resource.clone(),
            ResourceProgress {
                install: InstallProgress::Ready,
                removal: RemovalProgress::Draining,
                version: drain.resource_version,
                active_attempt: Some(drain.provenance.attempt_id),
                diagnostic: None,
            },
        )]),
    };
    let pending =
        PendingInstanceDrainClosure::new(snapshot.clone(), current, target.clone(), drain.clone())
            .unwrap();
    let admission = DeploymentAdmission {
        disposition: AdmissionDisposition::Resume,
        snapshot,
        receipt: CommandReceipt {
            command: current,
            deployment_id: intent.id(),
            input_hash: ContentHash::digest(b"controlled fresh admission"),
            lifecycle: DeploymentLifecycle::Removing,
            deployment_version: 9,
            event: event_application::CommittedMutation {
                event_id: Uuid::from_u128(90),
                scope_kind: "project".into(),
                scope_id: intent.project_id().as_uuid(),
                cursor: 1,
                aggregate_version: 1,
            },
        },
    };
    PendingFixture {
        admission,
        target,
        drain,
        pending,
    }
}

pub(super) fn continuation(exact: &PendingFixture) -> RemoveContinuation {
    RemoveContinuation {
        claim: exact.drain.clone(),
        closure: None,
    }
}

pub(super) fn closure(exact: &PendingFixture) -> OriginalInstanceClosure {
    let request = PreparedInstanceClosureRequest::new(
        exact.admission.snapshot.intent.clone(),
        exact.target.clone(),
        exact.drain.clone(),
        1,
    )
    .unwrap();
    OriginalInstanceClosure {
        publication_origin: crate::InstanceClosurePublicationOrigin::OriginalDrain,
        drain: exact.drain.clone(),
        removal_id: request.removal_id(),
        command_key: request.command_key(),
        expected_version: 1,
        actor: exact.drain.provenance.actor_id,
        request: exact.drain.provenance.request_id,
        input_hash: ContentHash::digest(b"controlled original closure"),
    }
}

pub(super) fn resumed(
    exact: &PendingFixture,
    version: u64,
) -> Result<ResumedPreparedInstanceClosureRequest, DeploymentError> {
    ResumedPreparedInstanceClosureRequest::new(
        exact.admission.receipt.command,
        exact.admission.snapshot.intent.clone(),
        exact.target.clone(),
        exact.drain.clone(),
        exact.pending.clone(),
        version,
    )
}

pub(super) struct Unsupported;
#[async_trait]
impl PreparedInstanceClosureRepository for Unsupported {}
