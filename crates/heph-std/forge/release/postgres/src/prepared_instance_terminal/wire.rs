use crate::prepared_instance_safety::claim;
use recipe_application::{
    DeploymentError, DeploymentExecutionProfile, PreparedInstanceDeleteRequest,
    PreparedInstanceDetachRequest, PreparedInstanceSafetyTarget, RemoveResumeConfiguration,
};
use release_service::InstanceExecutionConfiguration;
use serde_json::{Value, json};

pub fn configuration_bytes(
    config: &InstanceExecutionConfiguration,
) -> Result<Vec<u8>, DeploymentError> {
    serde_json::to_vec(&config.canonical_json()).map_err(|_| DeploymentError::Serialization)
}
pub fn configured(
    config: &InstanceExecutionConfiguration,
    target: &PreparedInstanceSafetyTarget,
    resume: &RemoveResumeConfiguration,
) -> Result<(), DeploymentError> {
    let scope = config.provider_scope();
    let platform = target.prepared().source().platform();
    let namespace = scope.namespace();
    if target.provider_namespace() != namespace
        || target.host_id() != scope.host_id()
        || resume.namespace() != namespace
        || resume.host() != scope.host_id()
        || resume.profile() != DeploymentExecutionProfile::RuntimeNamedV1
        || resume.platform() != platform
        || platform.policy() != config.platform_policy()
        || platform.version() != config.platform_version()
    {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok(())
}
pub fn detach(
    request: &PreparedInstanceDetachRequest,
    config: &InstanceExecutionConfiguration,
    closure: &Value,
) -> Result<Value, DeploymentError> {
    configured(config, request.target(), request.configuration())?;
    Ok(
        json!({"version":1,"deployment":request.intent().id(),"project":request.intent().project_id(),
        "resource":request.effect().resource,"intent_hash":request.intent().input_hash(),
        "create_claim":claim(request.target().original())?,"prepared":request.target().prepared(),
        "closure":closure,"delete_or_detach_claim":claim(request.effect())?,
        "current_command":command(request.current_command()),"configuration":config.canonical_json(),
        "before_instance_version":null}),
    )
}
pub fn delete(
    request: &PreparedInstanceDeleteRequest,
    config: &InstanceExecutionConfiguration,
    closure: &Value,
) -> Result<Value, DeploymentError> {
    configured(config, request.target(), request.configuration())?;
    // Native143 also rejects the maximum: a bigint successor must be representable.
    if request.expected_instance_version() >= i64::MAX.cast_unsigned() {
        return Err(DeploymentError::StaleClaim);
    }
    Ok(
        json!({"version":1,"deployment":request.intent().id(),"project":request.intent().project_id(),
        "resource":request.effect().resource,"intent_hash":request.intent().input_hash(),
        "create_claim":claim(request.target().original())?,"prepared":request.target().prepared(),
        "closure":closure,"delete_or_detach_claim":claim(request.effect())?,
        "current_command":command(request.current_command()),"configuration":config.canonical_json(),
        "before_instance_version":request.expected_instance_version()}),
    )
}
fn command(command: recipe_application::CommandIdentity) -> Value {
    json!({"id":command.id(),"actor_id":command.actor_id(),"idempotency_id":command.idempotency_id(),
        "operation":command.operation()})
}
