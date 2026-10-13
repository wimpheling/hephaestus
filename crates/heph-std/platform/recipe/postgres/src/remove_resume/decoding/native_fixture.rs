//! Controlled checked model/wire values, never a genuine PG producer fixture.
use super::super::{ClosureRow, Roots, tests};
use identity_domain::{RequestId, UserId};
use recipe_application::{
    CommandIdentity, DeploymentExecutionPreparation, DeploymentExecutionProfile,
    DeploymentLifecycle, DeploymentOperation, DeploymentSnapshot, EffectClaim,
    PlatformPolicyObservation, RemoveResumeConfiguration, ResourceAction, SourceObservation,
};
use release_domain::{ContentHash, NetworkAccess, RuntimePolicy};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

pub struct Fixture {
    pub intent: recipe_application::DeploymentIntent,
    pub drain: EffectClaim,
    pub preparation: DeploymentExecutionPreparation,
    pub configuration: RemoveResumeConfiguration,
    pub snapshot: DeploymentSnapshot,
    pub command: CommandIdentity,
    pub pending: Value,
    pub state: crate::effect_state::State,
}
impl Fixture {
    pub fn new() -> Self {
        let (intent, drain, _) = tests::fixture();
        let policy = RuntimePolicy {
            vcpus: 1,
            memory_mib: 128,
            network: NetworkAccess::Disabled,
        };
        let platform =
            PlatformPolicyObservation::new(policy.clone(), "platform/v1".to_owned()).unwrap();
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
            platform.clone(),
            &[source],
        )
        .unwrap();
        let configuration = RemoveResumeConfiguration::new(
            "namespace".to_owned(),
            "host".to_owned(),
            platform,
            DeploymentExecutionProfile::RuntimeNamedV1,
        )
        .unwrap();
        let mut create = drain.clone();
        create.command = install;
        create.action = ResourceAction::Create;
        create.generation = 1;
        create.resource_version = 2;
        create.provenance = recipe_application::AttemptProvenance {
            attempt_id: recipe_application::DeploymentAttemptId::from_uuid(Uuid::from_u128(17))
                .unwrap(),
            actor_id: install.actor_id(),
            request_id: RequestId::from_uuid(Uuid::from_u128(18)),
        };
        let state = crate::effect_state::State {
            install: recipe_application::InstallProgress::Ready,
            removal: recipe_application::RemovalProgress::Draining,
            version: 3,
            active_attempt: Some(drain.provenance.attempt_id.as_uuid()),
            diagnostic: None,
            generation: 2,
            last_action: Some(ResourceAction::Create),
        };
        let snapshot = DeploymentSnapshot {
            intent: intent.clone(),
            lifecycle: DeploymentLifecycle::Removing,
            version: 7,
            resources: BTreeMap::from([(drain.resource.clone(), state.progress().unwrap())]),
        };
        let pending = json!({"resource":drain.resource,"drain_claim":crate::execution_rows::ClaimWire::new(&drain).unwrap(),
            "actor":drain.command.actor_id(),"idempotency":drain.command.idempotency_id(),
            "create_claim":crate::execution_rows::ClaimWire::new(&create).unwrap(),"create_actor":install.actor_id(),
            "create_idempotency":install.idempotency_id(),"prepared":preparation.instances()[0],"observed_progress":state});
        Self {
            intent,
            drain,
            preparation,
            configuration,
            snapshot,
            command: command(30, 40, DeploymentOperation::Remove),
            pending,
            state,
        }
    }
    pub fn roots(&self) -> Roots<'_> {
        Roots {
            intent: &self.intent,
            preparation: &self.preparation,
            config: &self.configuration,
        }
    }
    pub fn claims(&self) -> Value {
        json!([{"claim":crate::execution_rows::ClaimWire::new(&self.drain).unwrap(),
        "actor":self.drain.command.actor_id(),"idempotency":self.drain.command.idempotency_id()}])
    }
    pub fn data(&self) -> Value {
        json!({"claims":self.claims(),"closures":[],"pending":[self.pending]})
    }
    pub fn closed(&self) -> ClosureRow {
        let (_, _, closure) = tests::fixture();
        let mut row = tests::row(&self.drain, &closure);
        row.actor = self.command.actor_id().as_uuid();
        row.request = Uuid::from_u128(50);
        row.source["publication"] = json!({"kind":"qualified_fresh_remove","command_id":self.command.id(),
            "actor":self.command.actor_id(),"idempotency":self.command.idempotency_id(),"admission_request":Uuid::from_u128(41),
            "publication_request":row.request,"admission_version":7,"pending":self.pending});
        row
    }
    pub fn closed_data(&self, row: &ClosureRow) -> Value {
        json!({"claims":self.claims(),"closures":[row.source]})
    }
    pub fn recover(&mut self, version: i64) {
        self.state.version = version;
        self.state.removal = recipe_application::RemovalProgress::RecoveryRequired;
        self.state.diagnostic = Some(recipe_application::DiagnosticCode::DrainPending);
        self.snapshot.lifecycle = DeploymentLifecycle::RecoveryRequired;
        self.snapshot.version += 1;
        self.snapshot
            .resources
            .insert(self.drain.resource.clone(), self.state.progress().unwrap());
    }
    pub fn recovery(&self) -> Value {
        json!([{"resource":self.drain.resource,
        "drain_claim":crate::execution_rows::ClaimWire::new(&self.drain).unwrap(),"observed_progress":self.state}])
    }
}
pub fn command(actor: u128, idempotency: u128, operation: DeploymentOperation) -> CommandIdentity {
    CommandIdentity::from_recorded(
        UserId::from_uuid(Uuid::from_u128(actor)),
        RequestId::from_uuid(Uuid::from_u128(idempotency)),
        operation,
    )
    .unwrap()
}
