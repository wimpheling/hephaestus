use super::fixture::{self, SOURCE};
use crate::{
    CommandIdentity, DeploymentExecutionPreparation, DeploymentExecutionProfile,
    DeploymentOperation, PlannedResourceIdentity,
};

#[test]
fn complete_imports_preserve_v1_identities_and_defaulted_parameters() {
    let intent = fixture::intent(SOURCE, 1);
    let original_hash = intent.input_hash();
    let prepared = fixture::prepared();
    assert_eq!(prepared.instances().len(), 2);
    assert_eq!(prepared.version(), 1);
    assert_eq!(prepared.intent_hash(), original_hash);
    assert_eq!(intent.input_hash(), original_hash);
    assert_eq!(prepared.instances()[0].resource().as_str(), "reader");
    for import in prepared.instances() {
        assert_eq!(
            intent.resources()[import.resource()].identity(),
            PlannedResourceIdentity::Instance {
                id: import.instance_id(),
                revision_id: import.revision_id()
            }
        );
        assert_eq!(import.release_pin(), fixture::pin());
        assert_eq!(import.selected_policy(), &fixture::policy(128));
        assert_eq!(import.platform_policy(), &fixture::policy(512));
        assert_eq!(import.platform_policy_version(), "platform/v1");
        assert_eq!(import.project_id(), intent.project_id());
        assert_eq!(
            import.name().as_str(),
            format!("recipe-{}", import.instance_id().as_uuid())
        );
        assert_eq!(
            import.parameters()[&release_domain::ParameterName::parse("message").unwrap()],
            release_domain::ParameterValue::String("hello".to_owned())
        );
        let slot = &import.volumes()[0];
        let volume_name = if import.resource().as_str() == "reader" {
            "read_data"
        } else {
            "data"
        };
        let PlannedResourceIdentity::Volume { id: data_id, .. } = intent.resources()
            [&capability_domain::CapabilitySlotKey::parse(volume_name).unwrap()]
            .identity()
        else {
            panic!("volume");
        };
        assert_eq!(slot.slot().as_str(), "state");
        assert_eq!(slot.guest_path().as_str(), "/data");
        assert_eq!(
            slot.access_mode(),
            volume_domain::VolumeAccessMode::ReadWrite
        );
        assert_eq!(slot.volume_id(), data_id);
        assert_eq!(slot.grant_id().as_uuid().get_version_num(), 5);
        assert_ne!(slot.grant_id().as_uuid(), import.operation_id());
        assert_eq!(import.operation_id().get_version_num(), 5);
    }
    assert_ne!(
        prepared.instances()[0].volumes()[0].grant_id(),
        prepared.instances()[1].volumes()[0].grant_id()
    );
    assert_ne!(
        prepared.instances()[0].command_key(),
        prepared.instances()[1].command_key()
    );
}

#[test]
fn stable_mapping_ignores_request_attempts_and_binds_original_command() {
    let first = fixture::prepared();
    let retry_command =
        CommandIdentity::from_identity(&fixture::identity(7, 900), DeploymentOperation::Install)
            .unwrap();
    let retry = DeploymentExecutionPreparation::new(
        &fixture::intent(SOURCE, 1),
        retry_command,
        first.profile(),
        first.platform().clone(),
        &[fixture::observation()],
    )
    .unwrap();
    assert_eq!(
        first.canonical_bytes().unwrap(),
        retry.canonical_bytes().unwrap()
    );
    let other_actor =
        CommandIdentity::from_identity(&fixture::identity(10, 8), DeploymentOperation::Install)
            .unwrap();
    let other = DeploymentExecutionPreparation::new(
        &fixture::intent(SOURCE, 1),
        other_actor,
        first.profile(),
        first.platform().clone(),
        &[fixture::observation()],
    )
    .unwrap();
    assert_ne!(first.input_hash().unwrap(), other.input_hash().unwrap());
    assert_ne!(
        first.instances()[0].command_key(),
        other.instances()[0].command_key()
    );
    assert_eq!(
        first.instances()[0].operation_id(),
        other.instances()[0].operation_id()
    );
    assert_eq!(
        first.instances()[0].volumes()[0].grant_id(),
        other.instances()[0].volumes()[0].grant_id()
    );
    let other_deployment = DeploymentExecutionPreparation::new(
        &fixture::intent(SOURCE, 3),
        fixture::command(),
        DeploymentExecutionProfile::RuntimeNamedV1,
        fixture::platform("platform/v1"),
        &[fixture::observation()],
    )
    .unwrap();
    assert_ne!(
        first.instances()[0].operation_id(),
        other_deployment.instances()[0].operation_id()
    );
    assert_ne!(
        first.instances()[0].volumes()[0].grant_id(),
        other_deployment.instances()[0].volumes()[0].grant_id()
    );
}

#[test]
fn non_install_command_is_not_a_preparation_input() {
    let remove =
        CommandIdentity::from_identity(&fixture::identity(7, 8), DeploymentOperation::Remove)
            .unwrap();
    assert!(
        DeploymentExecutionPreparation::new(
            &fixture::intent(SOURCE, 1),
            remove,
            DeploymentExecutionProfile::RuntimeNamedV1,
            fixture::platform("platform/v1"),
            &[fixture::observation()]
        )
        .is_err()
    );
}

#[test]
fn historical_command_identity_is_pure_checked_data() {
    let original = fixture::command();
    let restored = CommandIdentity::from_recorded(
        original.actor_id(),
        original.idempotency_id(),
        original.operation(),
    )
    .unwrap();
    assert_eq!(original, restored);
    assert!(
        CommandIdentity::from_recorded(
            identity_domain::UserId::from_uuid(uuid::Uuid::nil()),
            original.idempotency_id(),
            original.operation()
        )
        .is_err()
    );
    assert!(
        CommandIdentity::from_recorded(
            original.actor_id(),
            identity_domain::RequestId::from_uuid(uuid::Uuid::nil()),
            original.operation()
        )
        .is_err()
    );
}
