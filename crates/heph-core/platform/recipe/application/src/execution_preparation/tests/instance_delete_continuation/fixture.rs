//! Pure controlled history shapes; no PG admission or physical evidence.
use super::fixture;
use crate::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentError,
    DeploymentExecutionPreparation, DeploymentIntent, DeploymentOperation, EffectClaim,
    OriginalInstanceClosure, PreparedInstanceClosureRequest, PreparedInstanceDeleteRequest,
    PreparedInstanceSafetyTarget, RemoveResumeConfiguration, ResourceAction,
};
use release_domain::ContentHash;
use uuid::Uuid;

pub(super) struct DeleteFixture {
    pub(super) intent: DeploymentIntent,
    pub(super) target: PreparedInstanceSafetyTarget,
    pub(super) closure: OriginalInstanceClosure,
    pub(super) effect: EffectClaim,
    pub(super) current: CommandIdentity,
    pub(super) configuration: RemoveResumeConfiguration,
}
impl DeleteFixture {
    pub(super) fn new(action: ResourceAction) -> Self {
        let source = fixture::SOURCE.replace(
            "kind = \"instance\"",
            "kind = \"instance\"\nremoval = \"delete\"",
        );
        Self::for_intent(fixture::intent(&source, 1), action)
    }
    pub(super) fn for_intent(intent: DeploymentIntent, action: ResourceAction) -> Self {
        let prepared = DeploymentExecutionPreparation::new(
            &intent,
            fixture::command(),
            fixture::prepared().profile(),
            fixture::prepared().platform().clone(),
            &[fixture::observation()],
        )
        .unwrap()
        .instances()[0]
            .clone();
        let plan = &intent.resources()[prepared.resource()];
        let original = EffectClaim {
            command: fixture::command(),
            deployment_id: intent.id(),
            resource: prepared.resource().clone(),
            identity: plan.identity(),
            input_hash: plan.input_hash(),
            resource_version: 1,
            generation: 1,
            action: ResourceAction::Create,
            provenance: AttemptProvenance::new(&fixture::identity(7, 8), attempt(20)).unwrap(),
        };
        let target = PreparedInstanceSafetyTarget::new(
            original,
            prepared,
            "actual-owner:opaque".into(),
            "host-a".into(),
        )
        .unwrap();
        let actor = fixture::identity(9, 10);
        let drain = EffectClaim {
            command: CommandIdentity::from_identity(&actor, DeploymentOperation::Remove).unwrap(),
            action: ResourceAction::Drain,
            generation: 2,
            resource_version: 3,
            provenance: AttemptProvenance::new(&actor, attempt(50)).unwrap(),
            ..target.original().clone()
        };
        let request =
            PreparedInstanceClosureRequest::new(intent.clone(), target.clone(), drain.clone(), 2)
                .unwrap();
        let closure = OriginalInstanceClosure {
            publication_origin: crate::InstanceClosurePublicationOrigin::OriginalDrain,
            removal_id: request.removal_id(),
            command_key: request.command_key(),
            expected_version: 2,
            actor: actor.user_id,
            request: actor.request_id,
            input_hash: ContentHash::digest(b"controlled original closure"),
            drain,
        };
        let actor = fixture::identity(11, 12);
        let current = CommandIdentity::from_identity(&actor, DeploymentOperation::Remove).unwrap();
        let effect = EffectClaim {
            command: current,
            action,
            generation: 3,
            resource_version: 5,
            provenance: AttemptProvenance::new(&actor, attempt(51)).unwrap(),
            ..target.original().clone()
        };
        let configuration = RemoveResumeConfiguration::new(
            target.provider_namespace().into(),
            target.host_id().into(),
            fixture::prepared().platform().clone(),
            fixture::prepared().profile(),
        )
        .unwrap();
        Self {
            intent,
            target,
            closure,
            effect,
            current,
            configuration,
        }
    }
    pub(super) fn delete(
        &self,
        version: u64,
    ) -> Result<PreparedInstanceDeleteRequest, DeploymentError> {
        PreparedInstanceDeleteRequest::new(
            self.intent.clone(),
            self.target.clone(),
            self.closure.clone(),
            self.effect.clone(),
            self.current,
            self.configuration.clone(),
            version,
        )
    }
}
pub(super) fn attempt(value: u128) -> DeploymentAttemptId {
    DeploymentAttemptId::from_uuid(Uuid::from_u128(value)).unwrap()
}
pub(super) fn fresh_command() -> CommandIdentity {
    CommandIdentity::from_identity(&fixture::identity(13, 14), DeploymentOperation::Remove).unwrap()
}

impl DeleteFixture {
    pub(super) fn request_for(
        &self,
        current: CommandIdentity,
        before: u64,
    ) -> Result<PreparedInstanceDeleteRequest, DeploymentError> {
        PreparedInstanceDeleteRequest::new(
            self.intent.clone(),
            self.target.clone(),
            self.closure.clone(),
            self.effect.clone(),
            current,
            self.configuration.clone(),
            before,
        )
    }
    pub(super) fn change_owner(&mut self, namespace: &str, host: &str) {
        self.target = PreparedInstanceSafetyTarget::new(
            self.target.original().clone(),
            self.target.prepared().clone(),
            namespace.into(),
            host.into(),
        )
        .unwrap();
        self.configuration = RemoveResumeConfiguration::new(
            namespace.into(),
            host.into(),
            fixture::prepared().platform().clone(),
            fixture::prepared().profile(),
        )
        .unwrap();
    }
    pub(super) fn fresh_closure(&mut self) {
        let publisher =
            CommandIdentity::from_identity(&fixture::identity(15, 16), DeploymentOperation::Remove)
                .unwrap();
        let pending = crate::InstanceClosurePublicationProgress::new(
            self.closure.drain.clone(),
            crate::ResourceProgress {
                install: crate::InstallProgress::Ready,
                removal: crate::RemovalProgress::Draining,
                version: self.closure.drain.resource_version,
                active_attempt: Some(self.closure.drain.provenance.attempt_id),
                diagnostic: None,
            },
            self.closure.drain.generation,
            Some(ResourceAction::Create),
        )
        .unwrap();
        let publication = crate::QualifiedFreshInstanceClosurePublication::new(
            self.closure.drain.clone(),
            publisher,
            publisher.id(),
            fixture::identity(15, 16).request_id,
            fixture::identity(15, 18).request_id,
            9,
            pending,
        )
        .unwrap();
        self.closure.actor = publisher.actor_id();
        self.closure.request = publication.publication_request();
        self.closure.publication_origin =
            crate::InstanceClosurePublicationOrigin::QualifiedFreshRemove(Box::new(publication));
    }
}
