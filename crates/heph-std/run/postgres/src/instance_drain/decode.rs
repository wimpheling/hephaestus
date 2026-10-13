use std::collections::BTreeSet;

use run_domain::CancelRun;
use run_orchestrator::RepositoryError;
use runtime_types::RunId;
use serde_json::Value;
use uuid::Uuid;

use super::{InstanceDrainOriginal, InstanceDrainRunObservation};

#[derive(sqlx::FromRow)]
pub(super) struct Row {
    pub run_id: Uuid,
    pub cancel_json: Option<Value>,
    pub original: Value,
}

pub(super) fn validate_input(input: &InstanceDrainOriginal<'_>) -> Result<Uuid, RepositoryError> {
    if input.instance_id.as_uuid().is_nil()
        || input.drain_attempt.is_nil()
        || input.create_attempt.is_nil()
        || id(input.drain_claim, "attempt_id")? != input.drain_attempt
        || id(input.create_claim, "attempt_id")? != input.create_attempt
        || !input.prepared.is_object()
    {
        return Err(invalid());
    }
    id(input.create_claim, "command_id")?;
    id(input.drain_claim, "command_id")
}

pub(super) fn rows(
    rows: Vec<Row>,
    input: &InstanceDrainOriginal<'_>,
) -> Result<Vec<InstanceDrainRunObservation>, RepositoryError> {
    let command = validate_input(input)?;
    let mut runs = BTreeSet::new();
    let mut commands = BTreeSet::new();
    rows.into_iter()
        .map(|row| {
            if row.run_id.is_nil() || !runs.insert(row.run_id) {
                return Err(invalid());
            }
            let cancel = row
                .cancel_json
                .as_ref()
                .map(|value| decode_cancel(value, row.run_id))
                .transpose()?;
            if cancel.as_ref().is_some_and(|value| {
                value.reason != "instance removal requested" || !commands.insert(value.command_id)
            }) {
                return Err(invalid());
            }
            context(
                &row.original,
                input.drain_attempt,
                command,
                row.run_id,
                row.cancel_json.as_ref(),
            )?;
            Ok(InstanceDrainRunObservation {
                run_id: RunId::from_uuid(row.run_id),
                cancel,
                original: row.original,
            })
        })
        .collect()
}

fn context(
    original: &Value,
    attempt: Uuid,
    command: Uuid,
    run: Uuid,
    cancel: Option<&Value>,
) -> Result<(), RepositoryError> {
    if original.get("origin").is_none() {
        return closure(original, attempt, command);
    }
    if original["origin"] != "ordinary_cancel140" {
        return Err(invalid());
    }
    let expected: &[&str] = if cancel.is_some() {
        &[
            "origin",
            "controls",
            "cancels",
            "removal",
            "drain",
            "drain_cancel",
        ]
    } else {
        &["origin", "controls", "cancels", "removal"]
    };
    keys(original, expected)?;
    closure(&original["removal"], attempt, command)?;
    if let Some(cancel) = cancel {
        closure(&original["drain"], attempt, command)?;
        if original["drain"] != original["removal"] || &original["drain_cancel"] != cancel {
            return Err(invalid());
        }
    }
    let controls = original["controls"].as_array().ok_or_else(invalid)?;
    let cancels = original["cancels"].as_array().ok_or_else(invalid)?;
    if controls.is_empty() || cancels.len() != controls.len() + usize::from(cancel.is_some()) {
        return Err(invalid());
    }
    // Control metadata is opaque full native comparison data. Check only the
    // returned association; do not reproduce native authorization predicates.
    let mut ids = BTreeSet::new();
    for control in controls {
        let control_id = id(control, "id")?;
        if id(control, "run_id")? != run || !ids.insert(control_id) {
            return Err(invalid());
        }
    }
    let mut observed = BTreeSet::new();
    for value in cancels {
        let parsed = decode_cancel(value, run)?;
        if !observed.insert(parsed.command_id.as_uuid()) {
            return Err(invalid());
        }
    }
    if let Some(cancel) = cancel {
        let parsed = decode_cancel(cancel, run)?;
        if !observed.remove(&parsed.command_id.as_uuid()) {
            return Err(invalid());
        }
    }
    if observed != ids {
        return Err(invalid());
    }
    Ok(())
}

fn closure(value: &Value, attempt: Uuid, command: Uuid) -> Result<(), RepositoryError> {
    keys(
        value,
        &[
            "drain_attempt",
            "drain_command",
            "removal_id",
            "command_key",
            "input_hash",
            "expected_version",
            "creator",
            "request",
        ],
    )?;
    if id(value, "drain_attempt")? != attempt
        || id(value, "drain_command")? != command
        || value["expected_version"]
            .as_i64()
            .is_none_or(|version| version <= 0)
    {
        return Err(invalid());
    }
    for field in ["removal_id", "creator", "request"] {
        id(value, field)?;
    }
    for field in ["command_key", "input_hash"] {
        let bytes = value[field].as_array().ok_or_else(invalid)?;
        if bytes.len() != 32
            || bytes
                .iter()
                .any(|byte| byte.as_u64().is_none_or(|value| value > 255))
        {
            return Err(invalid());
        }
    }
    Ok(())
}

fn decode_cancel(value: &Value, run: Uuid) -> Result<CancelRun, RepositoryError> {
    keys(value, &["command_id", "run_id", "reason"])?;
    id(value, "command_id")?;
    if id(value, "run_id")? != run {
        return Err(invalid());
    }
    let cancel: CancelRun = serde_json::from_value(value.clone()).map_err(|_| invalid())?;
    // The existing DTO permits serde unknown fields. Exact native transport is
    // stricter: reject any dropped field or noncanonical UUID representation.
    if serde_json::to_value(&cancel).map_err(|_| invalid())? != *value {
        return Err(invalid());
    }
    Ok(cancel)
}

fn keys(value: &Value, expected: &[&str]) -> Result<(), RepositoryError> {
    let object = value.as_object().ok_or_else(invalid)?;
    if object.len() != expected.len() || expected.iter().any(|field| !object.contains_key(*field)) {
        return Err(invalid());
    }
    Ok(())
}

fn id(value: &Value, field: &str) -> Result<Uuid, RepositoryError> {
    let text = value[field].as_str().ok_or_else(invalid)?;
    let id = Uuid::parse_str(text).map_err(|_| invalid())?;
    if id.is_nil() || id.to_string() != text {
        return Err(invalid());
    }
    Ok(id)
}

const fn invalid() -> RepositoryError {
    RepositoryError::InvalidData("instance Drain observation contradicts exact original")
}
