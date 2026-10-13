//! Complete typed primary-key indexes; inherited JSONB/text rows stay exact.
use super::super::snapshots::{Row, Snapshot};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use uuid::{Uuid, Variant};

pub(super) const EVENT_FIELDS: [&str; 19] = [
    "id",
    "occurrence_id",
    "scope_kind",
    "scope_id",
    "cursor",
    "aggregate_type",
    "aggregate_id",
    "aggregate_version",
    "event_type",
    "schema_version",
    "change_kind",
    "safe_state",
    "related_id_one",
    "related_id_two",
    "actor_type",
    "actor_id",
    "request_id",
    "occurred_at",
    "retained_until",
];
pub(super) const OUTBOX_FIELDS: [&str; 8] = [
    "event_id",
    "subject",
    "created_at",
    "published_at",
    "dead_lettered_at",
    "attempts",
    "last_error",
    "terminal_reason",
];
pub(super) const SCOPE_FIELDS: [&str; 6] = [
    "scope_kind",
    "scope_id",
    "committed_cursor",
    "retained_from_cursor",
    "retention_interval",
    "updated_at",
];
pub(super) const AGGREGATE_FIELDS: [&str; 5] = [
    "scope_kind",
    "scope_id",
    "aggregate_type",
    "aggregate_id",
    "aggregate_version",
];

pub(super) fn indexed<'a>(
    snapshot: &'a Snapshot,
    table: &str,
    fields: &[&str],
    keys: &[&str],
) -> Result<BTreeMap<Vec<String>, &'a Row>, sqlx::Error> {
    let source = snapshot
        .tables()
        .get(table)
        .ok_or_else(|| sqlx::Error::Protocol("required event table absent".into()))?;
    require(
        source
            .columns
            .iter()
            .map(|c| c.name.as_str())
            .eq(fields.iter().copied()),
        "event table complete field identities changed",
    )?;
    let mut result = BTreeMap::new();
    for row in &source.rows {
        let object = row
            .json
            .as_object()
            .ok_or_else(|| sqlx::Error::Protocol("event row is not an object".into()))?;
        require(
            object.len() == fields.len() && fields.iter().all(|name| object.contains_key(*name)),
            "event row must retain its complete source field set",
        )?;
        let mut key = Vec::new();
        for name in keys {
            let value = if *name == "id" || name.ends_with("_id") {
                uuid(row, name)?.to_string()
            } else {
                text(row, name)?.into()
            };
            key.push(value);
        }
        require(
            result.insert(key, row).is_none(),
            "duplicate complete event-table primary key",
        )?;
    }
    Ok(result)
}

pub(super) fn appended<'a>(
    before: &Snapshot,
    after: &'a Snapshot,
    table: &str,
    fields: &[&str],
    keys: &[&str],
) -> Result<BTreeMap<Vec<String>, &'a Row>, sqlx::Error> {
    let original = indexed(before, table, fields, keys)?;
    let mut upgraded = indexed(after, table, fields, keys)?;
    require(
        before.tables()[table].columns == after.tables()[table].columns,
        "inherited event column metadata changed",
    )?;
    for (key, row) in original {
        require(
            upgraded.remove(&key) == Some(row),
            "prior complete JSONB/text event row changed or disappeared",
        )?;
    }
    Ok(upgraded)
}

pub(super) fn unchanged(
    before: &Snapshot,
    after: &Snapshot,
    name: &str,
) -> Result<(), sqlx::Error> {
    let old = before
        .tables()
        .get(name)
        .ok_or_else(|| sqlx::Error::Protocol("old event family missing".into()))?;
    let new = after
        .tables()
        .get(name)
        .ok_or_else(|| sqlx::Error::Protocol("new event family missing".into()))?;
    require(
        old.columns == new.columns && old.rows == new.rows,
        "undeclared event-family column or complete JSONB/text change",
    )
}

pub(super) fn field<'a>(row: &'a Row, name: &str) -> Result<&'a Value, sqlx::Error> {
    row.json
        .as_object()
        .and_then(|object| object.get(name))
        .ok_or_else(|| sqlx::Error::Protocol("required event field absent".into()))
}

pub(super) fn text<'a>(row: &'a Row, name: &str) -> Result<&'a str, sqlx::Error> {
    field(row, name)?
        .as_str()
        .ok_or_else(|| sqlx::Error::Protocol("event field is not text".into()))
}

pub(super) fn integer(row: &Row, name: &str) -> Result<i64, sqlx::Error> {
    field(row, name)?
        .as_i64()
        .ok_or_else(|| sqlx::Error::Protocol("event field is not bigint/integer".into()))
}

pub(super) fn uuid(row: &Row, name: &str) -> Result<Uuid, sqlx::Error> {
    let value = text(row, name)?;
    let id = Uuid::parse_str(value).map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
    require(
        id.to_string() == value,
        "event identity is not canonical UUID",
    )?;
    Ok(id)
}

pub(super) fn fresh(id: Uuid, reserved: &mut BTreeSet<Uuid>) -> Result<(), sqlx::Error> {
    require(
        id.get_version_num() == 4 && id.get_variant() == Variant::RFC4122 && reserved.insert(id),
        "new event/occurrence UUID is not fresh distinct RFC4122v4",
    )
}

pub(super) fn require(condition: bool, message: &str) -> Result<(), sqlx::Error> {
    if condition {
        Ok(())
    } else {
        Err(sqlx::Error::Protocol(message.into()))
    }
}
