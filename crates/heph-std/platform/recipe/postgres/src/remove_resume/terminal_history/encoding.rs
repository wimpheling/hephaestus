//! Closed12 comparison JSON; full closure is independently read, never synthesized.
use super::Reader;
use crate::execution_rows::ClaimWire;
use recipe_application::{DeploymentError, PreparedInstanceDeleteRequest};
use serde_json::{Value, json};

pub(super) fn request(
    reader: &Reader,
    request: &PreparedInstanceDeleteRequest,
    closure: &Value,
) -> Result<Value, DeploymentError> {
    if request.configuration() != &reader.remove
        || request.target().provider_namespace() != reader.remove.namespace()
        || request.target().host_id() != reader.remove.host()
        || request.expected_instance_version() == 0
        || request.expected_instance_version() >= i64::MAX.cast_unsigned()
    {
        return Err(DeploymentError::IntentMismatch);
    }
    let command = request.current_command();
    Ok(
        json!({"version":1,"project":request.intent().project_id(),"deployment":request.intent().id(),
        "intent_hash":request.intent().input_hash(),"resource":request.effect().resource,
        "create_claim":ClaimWire::new(request.target().original())?,"prepared":request.target().prepared(),
        "closure":closure,"delete_or_detach_claim":ClaimWire::new(request.effect())?,
        "current_command":{"id":command.id(),"actor_id":command.actor_id(),"idempotency_id":command.idempotency_id(),"operation":command.operation()},
        "configuration":reader.configuration.canonical_json(),"before_instance_version":request.expected_instance_version()}),
    )
}
pub(super) fn configuration(reader: &Reader) -> Result<Vec<u8>, DeploymentError> {
    serde_json::to_vec(&reader.configuration.canonical_json())
        .map_err(|_| DeploymentError::Serialization)
}
