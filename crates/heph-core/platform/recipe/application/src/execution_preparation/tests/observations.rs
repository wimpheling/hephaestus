use super::fixture::{self, SOURCE};
use crate::{
    DeploymentError, DeploymentExecutionPreparation, DeploymentExecutionProfile,
    PlatformPolicyObservation, SourceObservation,
};

#[test]
fn exact_configuration_drift_holds_without_changing_original_bytes() {
    let prepared = fixture::prepared();
    let original = prepared.canonical_bytes().unwrap();
    for changed in [
        fixture::platform("platform/v2"),
        PlatformPolicyObservation::new(fixture::policy(1024), "platform/v1".to_owned()).unwrap(),
    ] {
        assert!(matches!(
            prepared.validate_configuration(DeploymentExecutionProfile::RuntimeNamedV1, &changed),
            Err(DeploymentError::InputConflict)
        ));
    }
    prepared
        .validate_configuration(prepared.profile(), prepared.platform())
        .unwrap();
    assert_eq!(prepared.canonical_bytes().unwrap(), original);
}

#[test]
fn zero_instance_graph_still_fingerprints_global_platform() {
    let source = SOURCE
        .split("[[resources]]\nkind = \"instance\"")
        .next()
        .unwrap();
    let intent = fixture::intent(source, 1);
    let prepare = |version| {
        DeploymentExecutionPreparation::new(
            &intent,
            fixture::command(),
            DeploymentExecutionProfile::RuntimeNamedV1,
            fixture::platform(version),
            &[],
        )
        .unwrap()
    };
    let first = prepare("platform/v1");
    assert!(first.instances().is_empty());
    assert_eq!(first.platform().version(), "platform/v1");
    assert_ne!(
        first.input_hash().unwrap(),
        prepare("platform/v2").input_hash().unwrap()
    );
}

#[test]
fn source_coverage_is_complete_unique_and_has_exact_global_platform() {
    let intent = fixture::intent(SOURCE, 1);
    for sources in [vec![], vec![fixture::observation(), fixture::observation()]] {
        assert!(
            DeploymentExecutionPreparation::new(
                &intent,
                fixture::command(),
                DeploymentExecutionProfile::RuntimeNamedV1,
                fixture::platform("platform/v1"),
                &sources
            )
            .is_err()
        );
    }
    assert!(
        DeploymentExecutionPreparation::new(
            &intent,
            fixture::command(),
            DeploymentExecutionProfile::RuntimeNamedV1,
            fixture::platform("platform/v2"),
            &[fixture::observation()]
        )
        .is_err()
    );
    let volume_only = SOURCE
        .split("[[resources]]\nkind = \"instance\"")
        .next()
        .unwrap();
    assert!(
        DeploymentExecutionPreparation::new(
            &fixture::intent(volume_only, 1),
            fixture::command(),
            DeploymentExecutionProfile::RuntimeNamedV1,
            fixture::platform("platform/v1"),
            &[fixture::observation()]
        )
        .is_err()
    );
}

#[test]
fn source_policy_image_and_contract_hash_are_fingerprinted_outside_v1_intent() {
    let intent = fixture::intent(SOURCE, 1);
    let before = intent.input_hash();
    let first = fixture::prepared();
    let image = format!("registry.example/sqlite@sha256:{}", "b".repeat(64));
    for source in [
        SourceObservation::new(
            fixture::pin(),
            release_domain::ContentHash::digest(b"changed contract"),
            fixture::observation().image().as_str(),
            fixture::policy(128),
            fixture::platform("platform/v1"),
        )
        .unwrap(),
        SourceObservation::new(
            fixture::pin(),
            fixture::observation().runtime_contract_hash(),
            &image,
            fixture::policy(128),
            fixture::platform("platform/v1"),
        )
        .unwrap(),
        SourceObservation::new(
            fixture::pin(),
            fixture::observation().runtime_contract_hash(),
            fixture::observation().image().as_str(),
            fixture::policy(256),
            fixture::platform("platform/v1"),
        )
        .unwrap(),
    ] {
        let changed = DeploymentExecutionPreparation::new(
            &intent,
            fixture::command(),
            first.profile(),
            first.platform().clone(),
            &[source],
        )
        .unwrap();
        assert_ne!(changed.input_hash().unwrap(), first.input_hash().unwrap());
        assert_ne!(
            changed.instances()[0].input_hash().unwrap(),
            first.instances()[0].input_hash().unwrap()
        );
        assert_eq!(intent.input_hash(), before);
    }
}
