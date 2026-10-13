//! Controlled checked models only: no actual PG admission, worker or VM evidence.
use super::super::{wire, writing};
use forge_domain::ProjectId;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentExecutionPreparation,
    DeploymentExecutionProfile, DeploymentId, DeploymentIntent, DeploymentKey, DeploymentLifecycle,
    DeploymentOperation, DeploymentSnapshot, EffectClaim, InstallProgress,
    InstanceClosurePublicationOrigin, OriginalInstanceClosure, PendingInstanceDrainClosure,
    PlatformPolicyObservation, PreparedInstanceSafetyTarget,
    QualifiedFreshInstanceClosurePublication, RemovalProgress, ResourceAction, ResourceProgress,
    ResumedPreparedInstanceClosureRequest, SourceObservation,
};
use recipe_domain::{ReleaseCatalogEntry, ReleasePin, parse_recipe};
use release_domain::{ContentHash, NetworkAccess, ReleaseAgentId, ReleaseId, RuntimePolicy};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;
use volume_domain::{GuestMountPath, VolumeAccessMode, VolumeSlotDeclaration};

pub(super) struct Exact {
    pub target: PreparedInstanceSafetyTarget,
    pub request: ResumedPreparedInstanceClosureRequest,
    pub context: Value,
    pub closure: OriginalInstanceClosure,
    pub data: Value,
}
fn identity(actor: u128) -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        UserId::from_uuid(Uuid::from_u128(actor)),
        "test",
        "manager",
        json!({}),
        RequestId::from_uuid(Uuid::from_u128(actor + 100)),
    )
    .with_idempotency_id(RequestId::from_uuid(Uuid::from_u128(actor + 200)))
}
fn policy(memory_mib: u32) -> RuntimePolicy {
    RuntimePolicy {
        vcpus: 1,
        memory_mib,
        network: NetworkAccess::Disabled,
    }
}
impl Exact {
    pub fn new() -> Self {
        let (intent, whole) = prepared();
        let (target, drain) = originals(&intent, &whole);
        let install = target.original().command;
        let command =
            CommandIdentity::from_identity(&identity(11), DeploymentOperation::Remove).unwrap();
        let progress = ResourceProgress {
            install: InstallProgress::Ready,
            removal: RemovalProgress::Draining,
            version: 3,
            active_attempt: Some(drain.provenance.attempt_id),
            diagnostic: None,
        };
        let snapshot = DeploymentSnapshot {
            intent: intent.clone(),
            lifecycle: DeploymentLifecycle::Removing,
            version: 9,
            resources: BTreeMap::from([(drain.resource.clone(), progress)]),
        };
        let pending =
            PendingInstanceDrainClosure::new(snapshot, command, target.clone(), drain.clone())
                .unwrap();
        let request = ResumedPreparedInstanceClosureRequest::new(
            command,
            intent,
            target.clone(),
            drain.clone(),
            pending,
            2,
        )
        .unwrap();
        let pending_json = json!({"resource":drain.resource,"drain_claim":writing::claim(&drain).unwrap(),
            "actor":drain.command.actor_id(),"idempotency":drain.command.idempotency_id(),
            "create_claim":writing::claim(target.original()).unwrap(),"create_actor":install.actor_id(),
            "create_idempotency":install.idempotency_id(),"prepared":target.prepared(),
            "observed_progress":{"install":"ready","removal":"draining","version":3,
                "active_attempt":drain.provenance.attempt_id,"diagnostic":null,"generation":2,"last_action":"create"}});
        let publication = QualifiedFreshInstanceClosurePublication::new(
            drain.clone(),
            command,
            command.id(),
            identity(11).request_id,
            identity(12).request_id,
            9,
            wire::pending(&pending_json, &target, &drain).unwrap(),
        )
        .unwrap();
        let closure = OriginalInstanceClosure {
            publication_origin: InstanceClosurePublicationOrigin::QualifiedFreshRemove(Box::new(
                publication,
            )),
            drain: drain.clone(),
            removal_id: request.removal_id(),
            command_key: request.command_key(),
            expected_version: 2,
            actor: command.actor_id(),
            request: identity(12).request_id,
            input_hash: ContentHash::digest(b"controlled publication"),
        };
        let data = json!({"drain":writing::claim(&drain).unwrap(),"actor":drain.command.actor_id(),
            "idempotency":drain.command.idempotency_id(),"removal_id":closure.removal_id,
            "command_key":closure.command_key,"expected_version":2,"creator":closure.actor,
            "request":closure.request,"input_hash":closure.input_hash,
            "publication":{"kind":"qualified_fresh_remove","command_id":command.id(),"actor":command.actor_id(),
                "idempotency":command.idempotency_id(),"admission_request":identity(11).request_id,
                "publication_request":closure.request,"admission_version":9,"pending":pending_json}});
        Self {
            target,
            request,
            context: json!({"claims":[],"closures":[],"pending":[pending_json]}),
            closure,
            data,
        }
    }
}

fn prepared() -> (DeploymentIntent, DeploymentExecutionPreparation) {
    let source = r#"
contract_version = 1
recipe_id = "sqlite"
recipe_version = "1.0.0"
[[resources]]
kind = "volume"
name = "data"
source = { type = "created", capacity_bytes = { source = "literal", value = 16777216 } }
[[resources]]
kind = "instance"
name = "writer"
release = { release_id = "10000000-0000-4000-8000-000000000001", release_agent_id = "10000000-0000-4000-8000-000000000002" }
volume_bindings = [{ slot = "state", resource = "data", guest_path = "/data", access_mode = "read_write" }]
"#;
    let pin = ReleasePin {
        release_id: ReleaseId::from_uuid(
            Uuid::parse_str("10000000-0000-4000-8000-000000000001").unwrap(),
        ),
        release_agent_id: ReleaseAgentId::from_uuid(
            Uuid::parse_str("10000000-0000-4000-8000-000000000002").unwrap(),
        ),
    };
    let declaration = parse_recipe(source.as_bytes()).unwrap();
    let catalog = ReleaseCatalogEntry {
        pin,
        published: true,
        parameters: vec![],
        volume_slots: vec![
            VolumeSlotDeclaration::new(
                capability_domain::CapabilitySlotKey::parse("state").unwrap(),
                GuestMountPath::parse("/data").unwrap(),
                VolumeAccessMode::ReadWrite,
                true,
                16_777_216,
            )
            .unwrap(),
        ],
        required_secret_slot_count: 0,
        capability_requirements: vec![],
    };
    let resolved = declaration
        .resolve(&BTreeMap::new(), &BTreeMap::new(), &[catalog])
        .unwrap();
    let intent = DeploymentIntent::new(
        DeploymentId::from_uuid(Uuid::from_u128(1)).unwrap(),
        ProjectId::from_uuid(Uuid::from_u128(2)),
        DeploymentKey::parse("sqlite").unwrap(),
        &declaration,
        &resolved,
    )
    .unwrap();
    let install =
        CommandIdentity::from_identity(&identity(7), DeploymentOperation::Install).unwrap();
    let platform = PlatformPolicyObservation::new(policy(512), "platform/v1".into()).unwrap();
    let observation = SourceObservation::new(
        pin,
        ContentHash::digest(b"controlled source"),
        &format!("registry.example/sqlite@sha256:{}", "a".repeat(64)),
        policy(128),
        platform.clone(),
    )
    .unwrap();
    let whole = DeploymentExecutionPreparation::new(
        &intent,
        install,
        DeploymentExecutionProfile::RuntimeNamedV1,
        platform,
        &[observation],
    )
    .unwrap();
    (intent, whole)
}

fn originals(
    intent: &DeploymentIntent,
    whole: &DeploymentExecutionPreparation,
) -> (PreparedInstanceSafetyTarget, EffectClaim) {
    let install = whole.command();
    let prepared = whole.instances()[0].clone();
    let plan = &intent.resources()[prepared.resource()];
    let create = EffectClaim {
        command: install,
        deployment_id: intent.id(),
        resource: prepared.resource().clone(),
        identity: plan.identity(),
        input_hash: plan.input_hash(),
        resource_version: 1,
        generation: 1,
        action: ResourceAction::Create,
        provenance: AttemptProvenance::new(
            &identity(7),
            DeploymentAttemptId::from_uuid(Uuid::from_u128(20)).unwrap(),
        )
        .unwrap(),
    };
    let target = PreparedInstanceSafetyTarget::new(
        create.clone(),
        prepared,
        "owner:opaque".into(),
        "host-a".into(),
    )
    .unwrap();
    let drain = EffectClaim {
        command: CommandIdentity::from_identity(&identity(9), DeploymentOperation::Remove).unwrap(),
        action: ResourceAction::Drain,
        generation: 2,
        resource_version: 3,
        provenance: AttemptProvenance::new(
            &identity(9),
            DeploymentAttemptId::from_uuid(Uuid::from_u128(50)).unwrap(),
        )
        .unwrap(),
        ..create
    };
    (target, drain)
}
