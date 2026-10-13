//! Controlled model/scheduling checks, not `PostgreSQL` or physical cleanup proof.

use super::super::{Mode, Request};
use capability_domain::CapabilitySlotKey;
use forge_domain::ProjectId;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentExecutionPreparation,
    DeploymentExecutionProfile, DeploymentId, DeploymentIntent, DeploymentKey, DeploymentOperation,
    EffectClaim, OriginalInstanceClosure, PlatformPolicyObservation,
    PreparedInstanceClosureRequest, PreparedInstanceSafetyTarget, RemoveDeployment, ResourceAction,
};
use recipe_domain::{ReleaseCatalogEntry, ReleasePin, parse_recipe};
use release_domain::{ContentHash, NetworkAccess, ReleaseAgentId, ReleaseId, RuntimePolicy};
use std::collections::BTreeMap;
use uuid::Uuid;

pub(in crate::prepared_instance_terminal) fn request(actor: u128, mode: Mode) -> Request {
    let identity = AuthenticatedIdentity::new(
        UserId::from_uuid(Uuid::from_u128(actor)),
        "controlled",
        "current",
        serde_json::json!({}),
        RequestId::from_uuid(Uuid::from_u128(actor + 10)),
    );
    let command = CommandIdentity::from_identity(&identity, DeploymentOperation::Remove).unwrap();
    Request {
        identity,
        remove: RemoveDeployment {
            command,
            deployment_id: DeploymentId::from_uuid(Uuid::from_u128(1)).unwrap(),
            expected_version: 3,
        },
        resource: CapabilitySlotKey::parse("worker").unwrap(),
        mode,
    }
}

fn preparation_model() -> (
    DeploymentIntent,
    PlatformPolicyObservation,
    recipe_application::SourceObservation,
) {
    // A checked comparison model, with no native admission, row or receipt claim.
    let declaration = parse_recipe(br#"contract_version = 1
recipe_id = "controlled"
recipe_version = "1.0.0"
[[resources]]
kind = "instance"
removal = "delete"
name = "worker"
release = { release_id = "10000000-0000-4000-8000-000000000001", release_agent_id = "10000000-0000-4000-8000-000000000002" }
"#).unwrap();
    let pin = ReleasePin {
        release_id: ReleaseId::from_uuid(
            Uuid::parse_str("10000000-0000-4000-8000-000000000001").unwrap(),
        ),
        release_agent_id: ReleaseAgentId::from_uuid(
            Uuid::parse_str("10000000-0000-4000-8000-000000000002").unwrap(),
        ),
    };
    let catalog = ReleaseCatalogEntry {
        pin,
        published: true,
        parameters: vec![],
        volume_slots: vec![],
        required_secret_slot_count: 0,
        capability_requirements: vec![],
    };
    let resolved = declaration
        .resolve(&BTreeMap::new(), &BTreeMap::new(), &[catalog])
        .unwrap();
    let intent = DeploymentIntent::new(
        DeploymentId::from_uuid(Uuid::from_u128(1)).unwrap(),
        ProjectId::from_uuid(Uuid::from_u128(2)),
        DeploymentKey::parse("controlled").unwrap(),
        &declaration,
        &resolved,
    )
    .unwrap();
    let policy = RuntimePolicy {
        vcpus: 1,
        memory_mib: 128,
        network: NetworkAccess::Disabled,
    };
    let platform = PlatformPolicyObservation::new(policy.clone(), "controlled/v1".into()).unwrap();
    let source = recipe_application::SourceObservation::new(
        pin,
        ContentHash::digest(b"model"),
        &format!("registry.example/controlled@sha256:{}", "a".repeat(64)),
        policy,
        platform.clone(),
    )
    .unwrap();
    (intent, platform, source)
}

pub(in crate::prepared_instance_terminal) fn model() -> (
    DeploymentIntent,
    PreparedInstanceSafetyTarget,
    OriginalInstanceClosure,
) {
    let (intent, platform, source) = preparation_model();
    let install = request(7, Mode::Original);
    let install_command =
        CommandIdentity::from_identity(&install.identity, DeploymentOperation::Install).unwrap();
    let preparation = DeploymentExecutionPreparation::new(
        &intent,
        install_command,
        DeploymentExecutionProfile::RuntimeNamedV1,
        platform,
        &[source],
    )
    .unwrap();
    let prepared = preparation.instances()[0].clone();
    let plan = &intent.resources()[prepared.resource()];
    let claim = EffectClaim {
        command: install_command,
        deployment_id: intent.id(),
        resource: prepared.resource().clone(),
        identity: plan.identity(),
        input_hash: plan.input_hash(),
        resource_version: 1,
        generation: 1,
        action: ResourceAction::Create,
        provenance: AttemptProvenance::new(
            &install.identity,
            DeploymentAttemptId::from_uuid(Uuid::from_u128(20)).unwrap(),
        )
        .unwrap(),
    };
    let target = PreparedInstanceSafetyTarget::new(
        claim,
        prepared,
        "controlled-owner".into(),
        "host-a".into(),
    )
    .unwrap();
    let manager = request(9, Mode::Original);
    let drain = EffectClaim {
        command: manager.remove.command,
        resource_version: 3,
        generation: 2,
        action: ResourceAction::Drain,
        provenance: AttemptProvenance::new(
            &manager.identity,
            DeploymentAttemptId::from_uuid(Uuid::from_u128(30)).unwrap(),
        )
        .unwrap(),
        ..target.original().clone()
    };
    let closed =
        PreparedInstanceClosureRequest::new(intent.clone(), target.clone(), drain.clone(), 1)
            .unwrap();
    let closure = OriginalInstanceClosure {
        publication_origin: recipe_application::InstanceClosurePublicationOrigin::OriginalDrain,
        drain,
        removal_id: closed.removal_id(),
        command_key: closed.command_key(),
        expected_version: 1,
        actor: manager.identity.user_id,
        request: manager.identity.request_id,
        input_hash: ContentHash::digest(b"controlled closure; not a persisted fingerprint"),
    };
    (intent, target, closure)
}

pub(in crate::prepared_instance_terminal) fn delete()
-> recipe_application::PreparedInstanceDeleteRequest {
    let (intent, target, closure) = model();
    let caller = request(9, Mode::Original);
    let claim = EffectClaim {
        action: ResourceAction::Delete,
        generation: 4,
        resource_version: 7,
        provenance: AttemptProvenance::new(
            &caller.identity,
            DeploymentAttemptId::from_uuid(Uuid::from_u128(40)).unwrap(),
        )
        .unwrap(),
        ..closure.drain.clone()
    };
    let config = recipe_application::RemoveResumeConfiguration::new(
        target.provider_namespace().to_owned(),
        target.host_id().to_owned(),
        target.prepared().source().platform().clone(),
        DeploymentExecutionProfile::RuntimeNamedV1,
    )
    .unwrap();
    recipe_application::PreparedInstanceDeleteRequest::new(
        intent,
        target,
        closure,
        claim,
        caller.remove.command,
        config,
        3,
    )
    .unwrap()
}
