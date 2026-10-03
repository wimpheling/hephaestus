use release_domain::{AgentInstanceId, AgentInstanceRevisionId, ReleaseCommandKey};
use runtime_types::{CommandId, RunId};
use uuid::Uuid;

use super::{
    ActivateInstance, InstanceActivationAdmission, InstanceActivationId, InstanceExecutionError,
    InstanceInvocationAdmission, InstanceInvocationId, InvokeInstance,
};

struct Fixture {
    key: ReleaseCommandKey,
    activation: InstanceActivationId,
    invocation: InstanceInvocationId,
    instance: AgentInstanceId,
    revision: AgentInstanceRevisionId,
    run: RunId,
    start: CommandId,
}

fn fixture() -> Fixture {
    Fixture {
        key: ReleaseCommandKey::derive("instance_execution_test", &[b"stable-input"]),
        activation: InstanceActivationId::from_uuid(Uuid::from_u128(1)).unwrap(),
        invocation: InstanceInvocationId::from_uuid(Uuid::from_u128(2)).unwrap(),
        instance: AgentInstanceId::from_uuid(Uuid::from_u128(3)),
        revision: AgentInstanceRevisionId::from_uuid(Uuid::from_u128(4)),
        run: RunId::from_uuid(Uuid::from_u128(5)),
        start: CommandId::from_uuid(Uuid::from_u128(6)),
    }
}

#[test]
fn dedicated_identifiers_reject_nil_and_preserve_exact_values() {
    assert_eq!(
        InstanceActivationId::from_uuid(Uuid::nil()),
        Err(InstanceExecutionError::InvalidIdentifier)
    );
    assert_eq!(
        InstanceInvocationId::from_uuid(Uuid::nil()),
        Err(InstanceExecutionError::InvalidIdentifier)
    );
    let data = fixture();
    assert_eq!(data.activation.as_uuid(), Uuid::from_u128(1));
    assert_eq!(data.invocation.as_uuid(), Uuid::from_u128(2));
}

#[test]
fn activation_requires_exact_creation_version() {
    let data = fixture();
    for version in [0, 2, u64::MAX] {
        assert_eq!(
            ActivateInstance::new(
                data.key,
                data.activation,
                data.instance,
                data.revision,
                version
            ),
            Err(InstanceExecutionError::InvalidVersion)
        );
    }
    let command =
        ActivateInstance::new(data.key, data.activation, data.instance, data.revision, 1).unwrap();
    assert_eq!(command.command_key(), data.key);
    assert_eq!(command.activation_id(), data.activation);
    assert_eq!(command.instance_id(), data.instance);
    assert_eq!(command.expected_revision_id(), data.revision);
    assert_eq!(command.expected_creation_version(), 1);
}

#[test]
fn activation_input_rejects_each_nil_consumer_identity() {
    let data = fixture();
    for (instance, revision) in [
        (AgentInstanceId::from_uuid(Uuid::nil()), data.revision),
        (
            data.instance,
            AgentInstanceRevisionId::from_uuid(Uuid::nil()),
        ),
    ] {
        assert_eq!(
            ActivateInstance::new(data.key, data.activation, instance, revision, 1),
            Err(InstanceExecutionError::InvalidIdentifier)
        );
    }
}

#[test]
fn activation_result_requires_exact_version_and_non_nil_identities() {
    let data = fixture();
    for version in [0, 1, 3, u64::MAX] {
        assert_eq!(
            InstanceActivationAdmission::new(
                data.activation,
                data.instance,
                data.revision,
                version
            ),
            Err(InstanceExecutionError::InvalidVersion)
        );
    }
    for (instance, revision) in [
        (AgentInstanceId::from_uuid(Uuid::nil()), data.revision),
        (
            data.instance,
            AgentInstanceRevisionId::from_uuid(Uuid::nil()),
        ),
    ] {
        assert_eq!(
            InstanceActivationAdmission::new(data.activation, instance, revision, 2),
            Err(InstanceExecutionError::InvalidIdentifier)
        );
    }
    let result =
        InstanceActivationAdmission::new(data.activation, data.instance, data.revision, 2).unwrap();
    assert_eq!(result.activation_id(), data.activation);
    assert_eq!(result.instance_id(), data.instance);
    assert_eq!(result.revision_id(), data.revision);
    assert_eq!(result.activated_version(), 2);
}

#[test]
fn invocation_preserves_the_complete_backend_correspondence() {
    let data = fixture();
    let command = InvokeInstance::new(
        data.key,
        data.invocation,
        data.run,
        data.start,
        data.instance,
        data.revision,
    )
    .unwrap();
    assert_eq!(command.command_key(), data.key);
    assert_eq!(command.invocation_id(), data.invocation);
    assert_eq!(command.run_id(), data.run);
    assert_eq!(command.start_command_id(), data.start);
    assert_eq!(command.instance_id(), data.instance);
    assert_eq!(command.expected_revision_id(), data.revision);
    let result = InstanceInvocationAdmission::new(
        data.invocation,
        data.instance,
        data.revision,
        data.run,
        data.start,
    )
    .unwrap();
    assert_eq!(result.invocation_id(), data.invocation);
    assert_eq!(result.instance_id(), data.instance);
    assert_eq!(result.revision_id(), data.revision);
    assert_eq!(result.run_id(), data.run);
    assert_eq!(result.start_command_id(), data.start);
}

#[test]
fn invocation_input_and_result_reject_each_nil_generic_identity() {
    let data = fixture();
    for (instance, revision, run, start) in [
        (
            AgentInstanceId::from_uuid(Uuid::nil()),
            data.revision,
            data.run,
            data.start,
        ),
        (
            data.instance,
            AgentInstanceRevisionId::from_uuid(Uuid::nil()),
            data.run,
            data.start,
        ),
        (
            data.instance,
            data.revision,
            RunId::from_uuid(Uuid::nil()),
            data.start,
        ),
        (
            data.instance,
            data.revision,
            data.run,
            CommandId::from_uuid(Uuid::nil()),
        ),
    ] {
        assert_eq!(
            InvokeInstance::new(data.key, data.invocation, run, start, instance, revision),
            Err(InstanceExecutionError::InvalidIdentifier)
        );
        assert_eq!(
            InstanceInvocationAdmission::new(data.invocation, instance, revision, run, start),
            Err(InstanceExecutionError::InvalidIdentifier)
        );
    }
}
