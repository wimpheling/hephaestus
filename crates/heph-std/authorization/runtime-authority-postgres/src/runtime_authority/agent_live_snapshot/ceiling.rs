use capability_domain::{
    AuthorizationSnapshot, AuthorizationSnapshotId, WorkloadKind, WorkloadPrincipal,
};
use runtime_authority::{LiveRunSnapshotPins, RuntimeAuthorityError};
use sqlx::{Postgres, Transaction};

use super::super::{
    common::storage,
    gateway_helpers::{SnapshotBindingRow, stored_binding},
};

pub async fn resolve(
    tx: &mut Transaction<'_, Postgres>,
    pins: LiveRunSnapshotPins,
    authorization_model_version: &str,
) -> Result<AuthorizationSnapshot, RuntimeAuthorityError> {
    let rows = sqlx::query_as::<_, SnapshotBindingRow>(
        "SELECT binding.id AS binding_id, requirement.id AS requirement_id,
                requirement.slot_key, requirement.resource_kind,
                requirement.required_operations, requirement.optional_operations,
                requirement.slot_required, requirement.normalized_hash AS requirement_hash,
                binding.resource_id, binding.granted_operations,
                binding.normalized_hash AS binding_hash, binding.authorization_model_version
         FROM agent_capability_bindings binding
         JOIN release_capability_requirements requirement ON requirement.id=binding.requirement_id
           AND requirement.release_agent_id=binding.release_agent_id
         WHERE binding.instance_revision_id=$1 AND binding.release_agent_id=$2
         ORDER BY binding.slot_key,binding.id",
    )
    .bind(pins.revision())
    .bind(pins.release_agent())
    .fetch_all(&mut **tx)
    .await
    .map_err(storage)?;
    let bindings = rows
        .into_iter()
        .map(|row| stored_binding(row, authorization_model_version))
        .collect::<Result<Vec<_>, _>>()?;
    AuthorizationSnapshot::new(
        AuthorizationSnapshotId::from_uuid(pins.run()),
        WorkloadPrincipal::new(
            WorkloadKind::AgentInstance,
            pins.instance(),
            pins.revision(),
        ),
        authorization_model_version,
        bindings,
    )
    .map_err(|_| RuntimeAuthorityError::Persistence)
}

pub async fn operations_authorized(
    tx: &mut Transaction<'_, Postgres>,
    snapshot: &AuthorizationSnapshot,
) -> Result<bool, RuntimeAuthorityError> {
    for binding in snapshot.bindings() {
        for operation in binding.granted_operations() {
            let relation = format!("agent_{}", operation.as_str());
            let allowed: bool = sqlx::query_scalar(
                "SELECT check_permission('agent_instance',$1::text,$2,$3,$4::text)=1",
            )
            .bind(snapshot.principal().id)
            .bind(relation)
            .bind(binding.resource().kind.as_str())
            .bind(binding.resource().id)
            .fetch_one(&mut **tx)
            .await
            .map_err(storage)?;
            if !allowed {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
