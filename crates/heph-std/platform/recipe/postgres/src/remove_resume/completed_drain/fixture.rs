//! Controlled comparison models only; no database, worker or physical evidence.
use crate::remove_resume::decoding::tests;
use crate::{effect_state::State, execution_rows::ClaimWire};
use identity_domain::{RequestId, UserId};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentExecutionPreparation,
    DeploymentExecutionProfile, DeploymentOperation, ExistingInstanceClosureRequest,
    InstallProgress, InstanceClosurePublicationOrigin, InstanceClosurePublicationProgress,
    PlatformPolicyObservation, PreparedInstanceSafetyTarget,
    QualifiedFreshInstanceClosurePublication, RemovalProgress, ResourceAction, SourceObservation,
};
use release_domain::{ContentHash, NetworkAccess, RuntimePolicy};
use serde_json::{Value, json};
use uuid::Uuid;

pub(super) struct Exact {
    pub request: ExistingInstanceClosureRequest,
    pub data: Value,
}
fn command(actor: u128, idempotency: u128, operation: DeploymentOperation) -> CommandIdentity {
    CommandIdentity::from_recorded(
        UserId::from_uuid(Uuid::from_u128(actor)),
        RequestId::from_uuid(Uuid::from_u128(idempotency)),
        operation,
    )
    .unwrap()
}
impl Exact {
    pub fn new(fresh: bool) -> Self {
        let (intent, drain, mut closure) = tests::fixture();
        let policy = runtime_policy();
        let platform =
            PlatformPolicyObservation::new(policy.clone(), "platform/v1".into()).unwrap();
        let pin = recipe_domain::ReleasePin {
            release_id: release_domain::ReleaseId::from_uuid(Uuid::from_u128(10)),
            release_agent_id: release_domain::ReleaseAgentId::from_uuid(Uuid::from_u128(11)),
        };
        let source = SourceObservation::new(
            pin,
            ContentHash::digest(b"controlled declaration"),
            &format!("registry.example/decode@sha256:{}", "a".repeat(64)),
            policy,
            platform.clone(),
        )
        .unwrap();
        let install = command(7, 8, DeploymentOperation::Install);
        let preparation = DeploymentExecutionPreparation::new(
            &intent,
            install,
            DeploymentExecutionProfile::RuntimeNamedV1,
            platform,
            &[source],
        )
        .unwrap();
        let create = recipe_application::EffectClaim {
            command: install,
            action: ResourceAction::Create,
            generation: 1,
            resource_version: 2,
            provenance: AttemptProvenance {
                attempt_id: DeploymentAttemptId::from_uuid(Uuid::from_u128(17)).unwrap(),
                actor_id: install.actor_id(),
                request_id: RequestId::from_uuid(Uuid::from_u128(18)),
            },
            ..drain.clone()
        };
        let target = PreparedInstanceSafetyTarget::new(
            create,
            preparation.instances()[0].clone(),
            "namespace".into(),
            "host".into(),
        )
        .unwrap();
        let state = State {
            install: InstallProgress::Ready,
            removal: RemovalProgress::Draining,
            version: 3,
            active_attempt: Some(drain.provenance.attempt_id.as_uuid()),
            diagnostic: None,
            generation: 2,
            last_action: Some(ResourceAction::Create),
        };
        let pending = json!({"resource":drain.resource,"drain_claim":ClaimWire::new(&drain).unwrap(),
            "actor":drain.command.actor_id(),"idempotency":drain.command.idempotency_id(),
            "create_claim":ClaimWire::new(target.original()).unwrap(),"create_actor":install.actor_id(),
            "create_idempotency":install.idempotency_id(),"prepared":target.prepared(),"observed_progress":state});
        let publisher = command(30, 40, DeploymentOperation::Remove);
        let publication = QualifiedFreshInstanceClosurePublication::new(
            drain.clone(),
            publisher,
            publisher.id(),
            RequestId::from_uuid(Uuid::from_u128(41)),
            RequestId::from_uuid(Uuid::from_u128(50)),
            7,
            InstanceClosurePublicationProgress::new(
                drain.clone(),
                state.progress().unwrap(),
                2,
                state.last_action,
            )
            .unwrap(),
        )
        .unwrap();
        let tag = json!({"kind":"qualified_fresh_remove","command_id":publisher.id(),"actor":publisher.actor_id(),
            "idempotency":publisher.idempotency_id(),"admission_request":publication.admission_request(),
            "publication_request":publication.publication_request(),"admission_version":7,"pending":pending});
        if fresh {
            closure.actor = publisher.actor_id();
            closure.request = publication.publication_request();
            closure.publication_origin =
                InstanceClosurePublicationOrigin::QualifiedFreshRemove(Box::new(publication));
        }
        let mut data = json!({"drain":ClaimWire::new(&drain).unwrap(),"actor":drain.command.actor_id(),
            "idempotency":drain.command.idempotency_id(),"removal_id":closure.removal_id,"command_key":closure.command_key,
            "expected_version":closure.expected_version,"creator":closure.actor,"request":closure.request,"input_hash":closure.input_hash});
        if fresh {
            data["publication"] = tag;
        }
        let request = ExistingInstanceClosureRequest::new(
            command(90, 91, DeploymentOperation::Remove),
            intent,
            target,
            closure,
        )
        .unwrap();
        Self { request, data }
    }
}

const fn runtime_policy() -> RuntimePolicy {
    RuntimePolicy {
        vcpus: 1,
        memory_mib: 128,
        network: NetworkAccess::Disabled,
    }
}
