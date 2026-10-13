//! Controlled model/scheduling checks, not `PostgreSQL` or physical cleanup proof.

use super::{Mode, PreparedInstanceDrainError, Request, admission, execution};
use capability_domain::CapabilitySlotKey;
use forge_domain::ProjectId;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentExecutionPreparation,
    DeploymentExecutionProfile, DeploymentId, DeploymentIntent, DeploymentKey, DeploymentOperation,
    EffectClaim, OriginalInstanceClosure, PlatformPolicyObservation,
    PreparedInstanceClosureRequest, PreparedInstanceSafetyObservation, PreparedInstanceSafetyState,
    PreparedInstanceSafetyTarget, RemoveDeployment, ResourceAction, ResourceProgress,
};
use recipe_domain::{ReleaseCatalogEntry, ReleasePin, parse_recipe};
use release_domain::{ContentHash, NetworkAccess, ReleaseAgentId, ReleaseId, RuntimePolicy};
use std::collections::BTreeMap;
use uuid::Uuid;

pub(super) fn request(actor: u128, mode: Mode) -> Request {
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
        expected_instance_version: 1,
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

fn model() -> (PreparedInstanceSafetyTarget, OriginalInstanceClosure) {
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
        PreparedInstanceClosureRequest::new(intent, target.clone(), drain.clone(), 1).unwrap();
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
    (target, closure)
}

#[test]
fn no_receipt_before_exact_drained_target_and_original_removal() {
    let (target, closure) = model();
    let mut observed = PreparedInstanceSafetyObservation {
        target: target.clone(),
        state: PreparedInstanceSafetyState::ClosedHeld,
        removal_id: Some(closure.removal_id),
        inventory: recipe_application::InstanceSafetyInventory::default(),
    };
    assert!(matches!(
        execution::require_drained(&observed, &target, closure.removal_id),
        Err(PreparedInstanceDrainError::Pending)
    ));
    observed.state = PreparedInstanceSafetyState::Drained;
    assert!(execution::require_drained(&observed, &target, closure.removal_id).is_ok());
    let wrong = release_domain::InstanceRemovalId::from_uuid(Uuid::from_u128(99));
    assert!(execution::require_drained(&observed, &target, wrong).is_err());
    let foreign_target = PreparedInstanceSafetyTarget::new(
        target.original().clone(),
        target.prepared().clone(),
        "foreign-owner".into(),
        "host-a".into(),
    )
    .unwrap();
    assert!(execution::require_drained(&observed, &foreign_target, closure.removal_id).is_err());
    observed.removal_id = None;
    assert!(execution::require_drained(&observed, &target, closure.removal_id).is_err());
}

#[test]
fn inactive_original_requires_durable_completed_observer_not_an_admitted_claim() {
    let (_, closure) = model();
    assert_eq!(
        admission::existing_disposition(None, &closure).unwrap(),
        admission::ExistingDisposition::CompletedObservationRequired
    );
    assert_eq!(
        admission::existing_disposition(Some(&closure.drain), &closure).unwrap(),
        admission::ExistingDisposition::Active
    );
    let mut changed = closure.drain.clone();
    changed.generation += 1;
    assert!(admission::existing_disposition(Some(&changed), &closure).is_err());
}

#[test]
fn fresh_manager_reconciliation_keeps_original_claim_and_provenance() {
    let (target, closure) = model();
    let original_request = request(9, Mode::Original);
    let mut original = admission::Original::new(original_request.clone());
    original.target = Some(target);
    original.claim = Some(closure.drain.clone());
    original.closure = Some(closure.clone());
    let fresh = request(11, Mode::Resume);
    admission::compare_request(&fresh, &original).unwrap();
    assert_eq!(original.request.remove, original_request.remove);
    assert_eq!(original.claim.as_ref(), Some(&closure.drain));
    let progress = ResourceProgress {
        install: recipe_application::InstallProgress::Ready,
        removal: recipe_application::RemovalProgress::Draining,
        version: closure.drain.resource_version,
        active_attempt: Some(closure.drain.provenance.attempt_id),
        diagnostic: None,
    };
    assert!(matches!(
        execution::completion(&original_request, &closure.drain, &progress, &mut original).unwrap(),
        execution::Completion::Original
    ));
    let execution::Completion::Reconcile(fresh_provenance) =
        execution::completion(&fresh, &closure.drain, &progress, &mut original).unwrap()
    else {
        panic!("new manager cannot complete as old actor")
    };
    assert_eq!(fresh_provenance.actor_id, fresh.identity.user_id);
    assert_ne!(
        fresh_provenance.attempt_id,
        closure.drain.provenance.attempt_id
    );
    assert_ne!(fresh_provenance.actor_id, closure.actor);
    assert_eq!(
        execution::completion(&fresh, &closure.drain, &progress, &mut original).unwrap(),
        execution::Completion::Reconcile(fresh_provenance),
    );
    assert_eq!(original.completions.len(), 2);
    let mut foreign = fresh;
    foreign.resource = CapabilitySlotKey::parse("foreign").unwrap();
    assert!(admission::compare_request(&foreign, &original).is_err());
}

fn rows() -> Vec<run_postgres::InstanceDrainRunObservation> {
    [1, 2]
        .into_iter()
        .map(|id| run_postgres::InstanceDrainRunObservation {
            run_id: runtime_types::RunId::from_uuid(Uuid::from_u128(id)),
            cancel: None,
            original: serde_json::json!({"controlled_scheduling_only":true}),
        })
        .collect()
}

#[tokio::test]
async fn complete_batch_rejection_prevents_every_delivery() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let deliveries = AtomicUsize::new(0);
    let rows = rows();
    let result = execution::deliver_after_qualification(
        async {
            // Simulates SAME-controller rejection of a foreign/unknown final row.
            Err(recipe_application::DeploymentError::IntentMismatch.into())
        },
        &rows,
        |_| async {
            deliveries.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    )
    .await;
    assert!(result.is_err());
    assert_eq!(deliveries.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn qualified_batch_deliveries_are_ordered_and_stop_on_first_error() {
    let observed = std::sync::Mutex::new(Vec::new());
    let rows = rows();
    let observations = &observed;
    let result =
        execution::deliver_after_qualification(async { Ok(()) }, &rows, |row| async move {
            observations.lock().unwrap().push(row.run_id);
            Err(PreparedInstanceDrainError::Pending)
        })
        .await;
    assert!(result.is_err());
    assert_eq!(*observed.lock().unwrap(), vec![rows[0].run_id]);
}

#[path = "retry_tests.rs"]
mod retry;

#[path = "pending_tests.rs"]
mod pending;
