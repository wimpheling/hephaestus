//! Closed native142 context decoding, separate from current/saved binding.
#[path = "decoding/inventory.rs"]
mod inventory;
#[path = "decoding/pending.rs"]
mod pending;
#[path = "decoding/publication.rs"]
mod publication;
#[path = "decoding/replay.rs"]
mod replay;
#[path = "decoding/targets.rs"]
mod targets;
#[path = "decoding/terminal.rs"]
mod terminal;
#[path = "decoding/wire.rs"]
mod wire;

use crate::{execution_rows::ClaimWire, hydration, repository_error};
use identity_domain::{RequestId, UserId};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentError, DeploymentId,
    DeploymentIntent, DeploymentOperation, EffectClaim, OriginalInstanceClosure,
    RemoveContinuation,
};
use release_domain::{InstanceRemovalId, ReleaseCommandKey};
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use std::collections::BTreeSet;
use uuid::Uuid;

pub struct Roots<'a> {
    pub intent: &'a DeploymentIntent,
    pub preparation: &'a recipe_application::DeploymentExecutionPreparation,
    pub config: &'a recipe_application::RemoveResumeConfiguration,
}
impl Roots<'_> {
    fn validate(&self) -> Result<(), DeploymentError> {
        self.preparation
            .validate_configuration(self.config.profile(), self.config.platform())?;
        let expected_deployment = self.intent.id();
        if self.preparation.deployment_id() != expected_deployment
            || self.preparation.project_id() != self.intent.project_id()
            || self.preparation.intent_hash() != self.intent.input_hash()
        {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(())
    }
}

fn original(
    value: &Value,
    field: &str,
    intent: &DeploymentIntent,
) -> Result<EffectClaim, DeploymentError> {
    claim(value, field, intent, DeploymentOperation::Remove)
}
fn claim(
    value: &Value,
    field: &str,
    intent: &DeploymentIntent,
    operation: DeploymentOperation,
) -> Result<EffectClaim, DeploymentError> {
    let stored = value
        .get(field)
        .ok_or(DeploymentError::ReconciliationRequired)?;
    let wire: ClaimWire =
        serde_json::from_value(stored.clone()).map_err(|_| DeploymentError::Serialization)?;
    if serde_json::to_value(&wire).map_err(|_| DeploymentError::Serialization)? != *stored {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let command = wire::command(value, operation)?;
    let resource = capability_domain::CapabilitySlotKey::parse(wire.resource)
        .map_err(|_| DeploymentError::IntentMismatch)?;
    let planned = intent
        .resources()
        .get(&resource)
        .ok_or(DeploymentError::IntentMismatch)?;
    if wire.command_id != command.id().as_uuid()
        || wire.actor_id != command.actor_id().as_uuid()
        || wire.deployment_id != intent.id().as_uuid()
        || wire.input_hash != planned.input_hash()
        || wire.identity
            != serde_json::to_value(planned.identity())
                .map_err(|_| DeploymentError::Serialization)?
        || wire.request_id.is_nil()
        || wire.resource_version == 0
        || wire.resource_version > i64::MAX.cast_unsigned()
        || wire.generation == 0
        || wire.generation > i64::MAX.cast_unsigned()
    {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok(EffectClaim {
        command,
        deployment_id: DeploymentId::from_uuid(wire.deployment_id)?,
        resource,
        identity: planned.identity(),
        input_hash: wire.input_hash,
        resource_version: wire.resource_version,
        generation: wire.generation,
        action: wire.action,
        provenance: AttemptProvenance {
            attempt_id: DeploymentAttemptId::from_uuid(wire.attempt_id)?,
            actor_id: UserId::from_uuid(wire.actor_id),
            request_id: RequestId::from_uuid(wire.request_id),
        },
    })
}

#[derive(sqlx::FromRow)]
struct ClosureRow {
    resource: String,
    source: Value,
    id: Uuid,
    key: Vec<u8>,
    expected: i64,
    actor: Uuid,
    request: Uuid,
    hash: Vec<u8>,
}

pub struct Decoded {
    pub continuations: Vec<RemoveContinuation>,
    pub closures: Vec<OriginalInstanceClosure>,
    pending: Vec<pending::Pending>,
    recovery: Vec<(EffectClaim, crate::effect_state::State)>,
    terminal: terminal::Native143,
}

pub async fn load(
    tx: &mut Transaction<'_, Postgres>,
    data: &Value,
    roots: &Roots<'_>,
) -> Result<Decoded, DeploymentError> {
    roots.validate()?;
    wire::context(data, roots.intent.resources().len())?;
    let rows: Vec<ClosureRow> = sqlx::query_as("SELECT item->>'resource' resource,item source,(item->'removal'->>'id')::uuid id,
        decode(substr(item->'inbox'->>'command_key',3),'hex') key,(item->'removal'->>'expected_version')::bigint expected,
        (item->'removal'->>'created_by')::uuid actor,(item->'removal'->>'request_id')::uuid request,
        decode(substr(item->'removal'->>'input_hash',3),'hex') hash FROM jsonb_array_elements($1->'closures') item")
        .bind(data).fetch_all(&mut **tx).await.map_err(repository_error)?;
    let mut decoded = decode(data, &rows, roots.intent, Some(roots))?;
    terminal::load(tx, &mut decoded.terminal, roots).await?;
    Ok(decoded)
}

fn decode(
    data: &Value,
    rows: &[ClosureRow],
    intent: &DeploymentIntent,
    roots: Option<&Roots<'_>>,
) -> Result<Decoded, DeploymentError> {
    if let Some(roots) = roots {
        roots.validate()?;
        if roots.intent != intent {
            return Err(DeploymentError::IntentMismatch);
        }
    }
    wire::context(data, intent.resources().len())?;
    let observed = wire::array(data, "closures", intent.resources().len())?;
    if observed.len() != rows.len() || rows.iter().any(|row| !observed.contains(&row.source)) {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let mut closures = Vec::new();
    let mut resources = BTreeSet::new();
    for row in rows {
        let mut fields = vec![
            "resource",
            "drain_claim",
            "actor",
            "idempotency",
            "removal",
            "inbox",
        ];
        if row.source.get("publication").is_some() {
            fields.push("publication");
        }
        wire::fields(&row.source, &fields)?;
        let drain = original(&row.source, "drain_claim", intent)?;
        if row.resource != drain.resource.as_str() || !resources.insert(row.resource.clone()) {
            return Err(DeploymentError::ReconciliationRequired);
        }
        let closure = OriginalInstanceClosure {
            publication_origin: publication::decode(row.source.get("publication"), &drain, roots)?,
            drain,
            removal_id: InstanceRemovalId::from_uuid(row.id),
            command_key: serde_json::from_value::<ReleaseCommandKey>(
                serde_json::to_value(&row.key).map_err(|_| DeploymentError::Serialization)?,
            )
            .map_err(|_| DeploymentError::Serialization)?,
            expected_version: hydration::unsigned(row.expected)?,
            actor: UserId::from_uuid(row.actor),
            request: RequestId::from_uuid(row.request),
            input_hash: hydration::digest(&row.hash)?,
        };
        closure.validate(&closure.drain)?;
        closures.push(closure);
    }
    let mut continuations = Vec::new();
    let mut resources = BTreeSet::new();
    for item in wire::array(data, "claims", intent.resources().len())? {
        wire::fields(item, &["claim", "actor", "idempotency"])?;
        let claim = original(item, "claim", intent)?;
        if !resources.insert(claim.resource.clone()) {
            return Err(DeploymentError::IntentMismatch);
        }
        let closure = closures
            .iter()
            .find(|closure| closure.drain.resource == claim.resource)
            .cloned();
        continuations.push(RemoveContinuation { claim, closure });
    }
    let mut result = Decoded {
        continuations,
        closures,
        pending: Vec::new(),
        recovery: Vec::new(),
        terminal: terminal::Native143::parse(data, intent.resources().len())?,
    };
    inventory::endpoints(&mut result, data, roots)?;
    Ok(result)
}

pub use inventory::{Current, current};
pub use replay::observe;
pub use replay::{compare as compare_replay, receipt as validate_receipt};

pub struct Saved<'a> {
    pub version: u64,
    pub lifecycle: &'a str,
    pub data: &'a Value,
}

pub async fn qualify_replay(
    tx: &mut Transaction<'_, Postgres>,
    roots: &Roots<'_>,
    snapshot: &recipe_application::DeploymentSnapshot,
    command: CommandIdentity,
    saved: &Saved<'_>,
    current_data: &Value,
) -> Result<inventory::Current, DeploymentError> {
    let original = load(tx, saved.data, roots).await?;
    validate_receipt(saved.lifecycle, &original)?;
    let current = load(tx, current_data, roots).await?;
    compare_replay(&original, &current, snapshot, saved.version)?;
    observe(tx, command, roots, &original, &current, current_data).await?;
    inventory::current(current, snapshot, command)
}

pub(super) async fn load_terminal_target(
    tx: &mut Transaction<'_, Postgres>,
    command: CommandIdentity,
    roots: &Roots<'_>,
    resource: &capability_domain::CapabilitySlotKey,
) -> Result<recipe_application::PreparedInstanceSafetyTarget, DeploymentError> {
    targets::load(tx, command, roots, resource).await
}

pub(super) use terminal::HistoryEntry;
pub(super) fn terminal_history_entries(data: &Value) -> Result<Vec<HistoryEntry>, DeploymentError> {
    terminal::history_entries(data)
}
pub(super) async fn terminal_command(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> Result<CommandIdentity, DeploymentError> {
    terminal::command(tx, id).await
}

pub fn storage(error: sqlx::Error) -> DeploymentError {
    match error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .as_deref()
    {
        Some("42501") => DeploymentError::AuthorizationDenied,
        Some("P1180" | "P1420" | "P1430") => DeploymentError::ReconciliationRequired,
        _ => repository_error(error),
    }
}

#[cfg(test)]
#[path = "decoding_tests.rs"]
pub(super) mod tests;

#[cfg(test)]
#[path = "decoding/native_tests.rs"]
mod native_tests;
