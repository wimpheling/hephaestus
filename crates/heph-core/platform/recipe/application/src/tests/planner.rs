use std::collections::BTreeMap;

use forge_domain::ProjectId;
use recipe_domain::ReleasePin;
use release_domain::{
    ContentHash, NetworkAccess, ParameterName, ParameterValue, ReleaseAgentId, ReleaseId,
    RuntimePolicy,
};
use uuid::Uuid;

use crate::{
    DeploymentError, DeploymentKey, PlanningRequest, PlatformPolicyObservation, SourceObservation,
};

fn policy(memory_mib: u32) -> RuntimePolicy {
    RuntimePolicy {
        vcpus: 1,
        memory_mib,
        network: NetworkAccess::Disabled,
    }
}

fn pin() -> ReleasePin {
    ReleasePin {
        release_id: ReleaseId::from_uuid(Uuid::from_u128(3)),
        release_agent_id: ReleaseAgentId::from_uuid(Uuid::from_u128(4)),
    }
}

#[test]
fn released_selection_is_exact_and_incompatible_platform_is_rejected() {
    let image = format!("registry.example/sqlite@sha256:{}", "a".repeat(64));
    let platform =
        PlatformPolicyObservation::new(policy(512), String::from("platform/v1")).unwrap();
    let observation = SourceObservation::new(
        pin(),
        ContentHash::digest(b"contract"),
        &image,
        policy(128),
        platform,
    )
    .unwrap();
    assert_eq!(observation.selected_policy(), &policy(128));
    assert_eq!(
        observation.image().digest().unwrap().as_str(),
        format!("sha256:{}", "a".repeat(64))
    );
    assert!(matches!(
        SourceObservation::new(
            pin(),
            ContentHash::digest(b"contract"),
            &image,
            policy(128),
            PlatformPolicyObservation::new(policy(64), String::from("platform/v2")).unwrap()
        ),
        Err(DeploymentError::IncompatiblePolicy)
    ));
    assert!(
        SourceObservation::new(
            pin(),
            ContentHash::digest(b"contract"),
            "sqlite:latest",
            policy(128),
            PlatformPolicyObservation::new(policy(512), String::from("platform/v1")).unwrap()
        )
        .is_err()
    );
}

#[test]
fn planning_request_rejects_oversized_inputs_and_nil_external_ids() {
    let project = ProjectId::from_uuid(Uuid::from_u128(2));
    let oversized = BTreeMap::from([(
        ParameterName::parse("capacity").unwrap(),
        ParameterValue::String("x".repeat(recipe_domain::MAX_VALUE_BYTES + 1)),
    )]);
    assert!(matches!(
        PlanningRequest::new(
            project,
            DeploymentKey::parse("database").unwrap(),
            super::VOLUME.as_bytes(),
            oversized,
            BTreeMap::new()
        ),
        Err(DeploymentError::InvalidPlanningInput)
    ));
    let external = BTreeMap::from([(
        super::key("data"),
        runtime_types::VolumeId::from_uuid(Uuid::nil()),
    )]);
    assert!(
        PlanningRequest::new(
            project,
            DeploymentKey::parse("database").unwrap(),
            super::VOLUME.as_bytes(),
            BTreeMap::new(),
            external
        )
        .is_err()
    );
    assert!(
        PlanningRequest::new(
            project,
            DeploymentKey::parse("database").unwrap(),
            &vec![b' '; recipe_domain::MAX_RECIPE_BYTES + 1],
            BTreeMap::new(),
            BTreeMap::new()
        )
        .is_err()
    );
}

#[test]
fn source_observations_do_not_change_frozen_v1_intent() {
    let request = PlanningRequest::new(
        ProjectId::from_uuid(Uuid::from_u128(2)),
        DeploymentKey::parse("database").unwrap(),
        super::VOLUME.as_bytes(),
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .unwrap();
    let declaration = recipe_domain::parse_recipe(super::VOLUME.as_bytes()).unwrap();
    assert_eq!(request.declaration().hash(), declaration.hash());
    assert_eq!(
        super::intent(request.declaration(), request.inputs()),
        super::intent(&declaration, &BTreeMap::new())
    );
    for version in ["", " bad", "bad\n", &"x".repeat(129)] {
        assert!(PlatformPolicyObservation::new(policy(128), version.to_owned()).is_err());
    }
}

#[test]
fn deployment_identity_is_stable_for_exact_project_and_key() {
    let request = |project: u128, key: &str| {
        PlanningRequest::new(
            ProjectId::from_uuid(Uuid::from_u128(project)),
            DeploymentKey::parse(key).unwrap(),
            super::VOLUME.as_bytes(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap()
    };
    let original = request(2, "database");
    assert_eq!(original.id(), request(2, "database").id());
    assert_ne!(original.id(), request(3, "database").id());
    assert_ne!(original.id(), request(2, "database-next").id());
    assert_ne!(original.id(), request(2, "Database").id());
    let changed = PlanningRequest::new(
        original.project_id(),
        original.key().clone(),
        super::VOLUME.as_bytes(),
        BTreeMap::from([(
            ParameterName::parse("capacity").unwrap(),
            ParameterValue::Integer(33_554_432),
        )]),
        BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        original.id(),
        changed.id(),
        "input changes cannot bypass the deployment tombstone"
    );
    assert_eq!(original.id().as_uuid().get_version_num(), 5);
}
