use super::derive;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentId, DeploymentIntent,
    DeploymentKey, DeploymentOperation, EffectClaim, ResourceAction,
};
use uuid::Uuid;

fn fixture() -> (DeploymentIntent, EffectClaim) {
    let source = "contract_version=1\nrecipe_id=\"data\"\nrecipe_version=\"1\"\n[[resources]]\nkind=\"volume\"\nname=\"data\"\nsource={type=\"created\",capacity_bytes={source=\"literal\",value=16777216}}\n";
    let parsed = recipe_domain::parse_recipe(source.as_bytes()).unwrap();
    let resolved = parsed
        .resolve(
            &std::collections::BTreeMap::new(),
            &std::collections::BTreeMap::new(),
            &[],
        )
        .unwrap();
    let intent = DeploymentIntent::new(
        DeploymentId::from_uuid(Uuid::new_v4()).unwrap(),
        forge_domain::ProjectId::from_uuid(Uuid::new_v4()),
        DeploymentKey::parse("mapped").unwrap(),
        &parsed,
        &resolved,
    )
    .unwrap();
    let identity = AuthenticatedIdentity::new(
        UserId::new(),
        "fixture",
        "fixture",
        serde_json::Value::Null,
        RequestId::new(),
    );
    let (name, resource) = intent.resources().first_key_value().unwrap();
    let claim = EffectClaim {
        command: CommandIdentity::from_identity(&identity, DeploymentOperation::Install).unwrap(),
        deployment_id: intent.id(),
        resource: name.clone(),
        identity: resource.identity(),
        input_hash: resource.input_hash(),
        resource_version: 1,
        generation: 1,
        action: ResourceAction::Create,
        provenance: AttemptProvenance::new(
            &identity,
            DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
        )
        .unwrap(),
    };
    (intent, claim)
}
#[test]
fn tagged_mapping_is_exact_stable_and_distinct_from_original_attempt() {
    let (intent, claim) = fixture();
    let prepared = derive(&intent, &claim).unwrap();
    assert_eq!(prepared, derive(&intent, &claim).unwrap());
    assert_ne!(
        prepared.creation.fields().effect_id,
        claim.provenance.attempt_id.as_uuid()
    );
    assert_ne!(
        prepared.creation.fields().operation_id.as_uuid(),
        prepared.creation.fields().effect_id
    );
    assert_ne!(
        prepared.creation.fields().scope_id.as_uuid(),
        claim.deployment_id.as_uuid()
    );
    let mut changed = claim;
    changed.provenance.attempt_id = DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap();
    let next = derive(&intent, &changed).unwrap();
    assert_eq!(
        next.creation.fields().scope_id,
        prepared.creation.fields().scope_id
    );
    assert_ne!(next.claim_digest, prepared.claim_digest);
    assert_ne!(
        next.creation.fields().effect_id,
        prepared.creation.fields().effect_id
    );
    assert_ne!(
        next.creation.fields().operation_id,
        prepared.creation.fields().operation_id
    );
}
#[test]
fn preparation_cannot_change_owned_identity_or_action() {
    let (intent, mut claim) = fixture();
    claim.action = ResourceAction::Retain;
    assert!(derive(&intent, &claim).is_err());
    claim.action = ResourceAction::Create;
    claim.input_hash = release_domain::ContentHash::digest(b"changed");
    assert!(derive(&intent, &claim).is_err());
}
