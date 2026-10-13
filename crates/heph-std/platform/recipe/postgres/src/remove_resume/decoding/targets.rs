//! Original target obtained through the existing current-manager native face.
use super::{Roots, claim};
use identity_domain::{RequestId, UserId};
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentOperation, PreparedInstanceSafetyTarget,
};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct Row {
    claim_json: Value,
    original_actor: Uuid,
    original_idempotency: Uuid,
    canonical_input: Vec<u8>,
    preparation_hash: Vec<u8>,
}

pub(super) async fn load(
    tx: &mut Transaction<'_, Postgres>,
    command: CommandIdentity,
    roots: &Roots<'_>,
    resource: &capability_domain::CapabilitySlotKey,
) -> Result<PreparedInstanceSafetyTarget, DeploymentError> {
    let row: Row = sqlx::query_as("SELECT claim_json,original_actor,original_idempotency,canonical_input,preparation_hash FROM load_prepared_original_target118($1,$2,$3,$4,$5)")
        .bind(command.id().as_uuid()).bind(roots.intent.id().as_uuid()).bind(resource.as_str())
        .bind(roots.intent.project_id().as_uuid()).bind(roots.intent.input_hash().as_bytes().as_slice())
        .fetch_one(&mut **tx).await.map_err(super::storage)?;
    let install = CommandIdentity::from_recorded(
        UserId::from_uuid(row.original_actor),
        RequestId::from_uuid(row.original_idempotency),
        DeploymentOperation::Install,
    )?;
    if install != roots.preparation.command()
        || row.canonical_input != roots.preparation.canonical_bytes()?
        || row.preparation_hash.as_slice() != roots.preparation.input_hash()?.as_bytes()
    {
        return Err(DeploymentError::IntentMismatch);
    }
    let create = claim(
        &json!({"claim":row.claim_json,"actor":row.original_actor,"idempotency":row.original_idempotency}),
        "claim",
        roots.intent,
        DeploymentOperation::Install,
    )?;
    if &create.resource != resource || create.action != recipe_application::ResourceAction::Create {
        return Err(DeploymentError::IntentMismatch);
    }
    let prepared = roots
        .preparation
        .instances()
        .iter()
        .find(|item| item.resource() == resource)
        .ok_or(DeploymentError::IntentMismatch)?;
    PreparedInstanceSafetyTarget::new(
        create,
        prepared.clone(),
        roots.config.namespace().to_owned(),
        roots.config.host().to_owned(),
    )
}
