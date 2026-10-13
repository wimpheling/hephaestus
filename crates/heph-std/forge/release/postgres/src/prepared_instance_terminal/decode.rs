use super::origin::fields;
use crate::prepared_instance_safety::SafetyRow;
use recipe_application::{
    DeploymentError, PreparedInstanceDeleteObservation, PreparedInstanceDeleteRequest,
};
use serde_json::Value;
use sqlx::types::Json;
use uuid::Uuid;

pub fn detach(value: &Value, request: &Value) -> Result<SafetyRow, DeploymentError> {
    fields(
        value,
        &[
            "version",
            "request",
            "birth_proven",
            "cleanup_ready",
            "removal_id",
            "inventory",
        ],
    )?;
    if value["version"] != 1
        || value["request"] != *request
        || value["birth_proven"] != true
        || value["cleanup_ready"] != true
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let removal = value["removal_id"]
        .as_str()
        .and_then(|id| id.parse::<Uuid>().ok())
        .filter(|id| !id.is_nil())
        .ok_or(DeploymentError::ReconciliationRequired)?;
    Ok(SafetyRow {
        birth_proven: true,
        removal_id: Some(removal),
        cleanup_ready: true,
        inventory: Json(
            serde_json::from_value(value["inventory"].clone())
                .map_err(|_| DeploymentError::ReconciliationRequired)?,
        ),
    })
}
pub(super) fn terminal(rows: &[(Uuid, i64)], before: u64) -> Result<(Uuid, u64), DeploymentError> {
    let [(operation, after)] = rows else {
        return Err(DeploymentError::ReconciliationRequired);
    };
    let after = u64::try_from(*after).map_err(|_| DeploymentError::ReconciliationRequired)?;
    let expected = before
        .checked_add(1)
        .filter(|value| *value <= i64::MAX.cast_unsigned())
        .ok_or(DeploymentError::StaleClaim)?;
    if operation.is_nil() || after != expected {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok((*operation, after))
}
pub fn deleted(
    request: &PreparedInstanceDeleteRequest,
    rows: &[(Uuid, i64)],
) -> Result<PreparedInstanceDeleteObservation, DeploymentError> {
    let (operation, after) = terminal(rows, request.expected_instance_version())?;
    PreparedInstanceDeleteObservation::new(request.clone(), operation, after)
}
