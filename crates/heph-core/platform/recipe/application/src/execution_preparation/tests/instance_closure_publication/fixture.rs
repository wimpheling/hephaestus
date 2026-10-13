//! Controlled checked comparison models, not native admission or PG evidence.
use super::super::{fixture, instance_safety};
use crate::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentLifecycle,
    DeploymentOperation, DeploymentSnapshot, EffectClaim, InstallProgress,
    InstanceClosurePublicationOrigin, InstanceClosurePublicationProgress, OriginalInstanceClosure,
    PreparedInstanceClosureRequest, PreparedInstanceSafetyTarget,
    QualifiedFreshInstanceClosurePublication, RemovalProgress, ResourceAction, ResourceProgress,
};
use release_domain::ContentHash;
use std::collections::BTreeMap;
use uuid::Uuid;

pub(super) struct Exact {
    pub(super) target: PreparedInstanceSafetyTarget,
    pub(super) drain: EffectClaim,
    pub(super) snapshot: DeploymentSnapshot,
    pub(super) publisher: CommandIdentity,
    pub(super) closure: OriginalInstanceClosure,
}
impl Exact {
    pub(super) fn new() -> Self {
        let target = instance_safety::target();
        let actor = fixture::identity(9, 10);
        let drain = EffectClaim {
            command: command(9, 10),
            action: ResourceAction::Drain,
            generation: 2,
            resource_version: 3,
            provenance: AttemptProvenance::new(
                &actor,
                DeploymentAttemptId::from_uuid(Uuid::from_u128(50)).unwrap(),
            )
            .unwrap(),
            ..target.original().clone()
        };
        let snapshot = DeploymentSnapshot {
            intent: fixture::intent(fixture::SOURCE, 1),
            lifecycle: DeploymentLifecycle::Removing,
            version: 9,
            resources: BTreeMap::from([(
                drain.resource.clone(),
                ResourceProgress {
                    install: InstallProgress::Ready,
                    removal: RemovalProgress::Draining,
                    version: 3,
                    active_attempt: Some(drain.provenance.attempt_id),
                    diagnostic: None,
                },
            )]),
        };
        let request = PreparedInstanceClosureRequest::new(
            snapshot.intent.clone(),
            target.clone(),
            drain.clone(),
            2,
        )
        .unwrap();
        let closure = OriginalInstanceClosure {
            publication_origin: InstanceClosurePublicationOrigin::OriginalDrain,
            removal_id: request.removal_id(),
            command_key: request.command_key(),
            expected_version: 2,
            actor: actor.user_id,
            request: actor.request_id,
            input_hash: ContentHash::digest(b"controlled original publication"),
            drain: drain.clone(),
        };
        Self {
            target,
            drain,
            snapshot,
            publisher: command(11, 12),
            closure,
        }
    }
    pub(super) fn progress(&self) -> InstanceClosurePublicationProgress {
        InstanceClosurePublicationProgress::new(
            self.drain.clone(),
            self.snapshot.resources[&self.drain.resource].clone(),
            self.drain.generation,
            Some(ResourceAction::Create),
        )
        .unwrap()
    }
    pub(super) fn publication(&self) -> QualifiedFreshInstanceClosurePublication {
        QualifiedFreshInstanceClosurePublication::new(
            self.drain.clone(),
            self.publisher,
            self.publisher.id(),
            fixture::identity(11, 12).request_id,
            fixture::identity(11, 15).request_id,
            self.snapshot.version,
            self.progress(),
        )
        .unwrap()
    }
    pub(super) fn fresh_closure(&self) -> OriginalInstanceClosure {
        let publication = self.publication();
        OriginalInstanceClosure {
            actor: publication.command().actor_id(),
            request: publication.publication_request(),
            publication_origin: InstanceClosurePublicationOrigin::QualifiedFreshRemove(Box::new(
                publication,
            )),
            ..self.closure.clone()
        }
    }
}
pub(super) fn command(actor: u128, request: u128) -> CommandIdentity {
    CommandIdentity::from_identity(
        &fixture::identity(actor, request),
        DeploymentOperation::Remove,
    )
    .unwrap()
}
