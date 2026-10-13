//! Exact added values and identities; original columns have a separate oracle.
use super::snapshots::{Phase, Row, Snapshot};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

mod qualification;
pub use qualification::{Qualification, Selection};

/// Derive sealed migration108 selections from BEFORE107 alone.
///
/// # Errors
/// Rejects the wrong phase or malformed relational identities/predicate values.
pub fn qualify(before: &Snapshot) -> Result<Qualification<'_>, sqlx::Error> {
    qualification::derive(before)
}

/// Check every old-table column delta and every old row's new values.
///
/// # Errors
/// Rejects malformed BEFORE data, undeclared columns, row identity drift or values.
pub fn assert_values<'a>(
    before: &'a Snapshot,
    after: &Snapshot,
) -> Result<Qualification<'a>, sqlx::Error> {
    require(
        before.phase() == Phase::Public107
            && matches!(
                after.phase(),
                Phase::Public137
                    | Phase::Public138
                    | Phase::Public139
                    | Phase::Public140
                    | Phase::Public141
                    | Phase::Public142
                    | Phase::Public143
            ),
        "added fields require107 before and137..143 after",
    )?;
    let qualification = qualify(before)?;
    column_sets(before, after)?;
    for table in [
        "runs",
        "agent_instance_volume_leases",
        "agent_instance_state_volumes",
        "recipe_execution_transitions",
        "agent_instance_removal_requests",
    ] {
        let original = rows(before, table)?;
        let upgraded = rows(after, table)?;
        require(
            original.keys().eq(upgraded.keys()),
            "added-field oracle found missing or extra old row identities",
        )?;
        for (id, row) in upgraded {
            let selection = qualification.selections().get(&id);
            let expected = if table == "agent_instance_volume_leases" {
                lease_values(selection)
            } else {
                defaults(
                    table,
                    matches!(after.phase(), Phase::Public142 | Phase::Public143),
                )
            };
            for (name, value) in expected {
                require(
                    field(row, &name)? == &value,
                    "old row added value differs from source-defined BEFORE oracle",
                )?;
            }
        }
    }
    Ok(qualification)
}

fn column_sets(before: &Snapshot, after: &Snapshot) -> Result<(), sqlx::Error> {
    for (name, original) in before.tables() {
        let upgraded = after
            .tables()
            .get(name)
            .ok_or_else(|| sqlx::Error::Protocol("old table disappeared".into()))?;
        let old: BTreeSet<_> = original.columns.iter().map(|c| c.name.as_str()).collect();
        let new: BTreeSet<_> = upgraded.columns.iter().map(|c| c.name.as_str()).collect();
        require(
            old.len() == original.columns.len()
                && new.len() == upgraded.columns.len()
                && old.is_subset(&new),
            "duplicate or removed old column names",
        )?;
        let expected = defaults(
            name,
            matches!(after.phase(), Phase::Public142 | Phase::Public143),
        );
        let added: BTreeSet<_> = new.difference(&old).copied().collect();
        require(
            added == expected.keys().map(String::as_str).collect(),
            "old table has an undeclared added-column set",
        )?;
    }
    Ok(())
}

fn defaults(table: &str, public142: bool) -> Map<String, Value> {
    let fields = match table {
        "runs" => json!({
            "volume_acquisition_closed_at": null, "volume_selection_snapshot": null,
            "vm_plan_admission_required": false, "execution_profile": "historical_unproven",
            "vm_provider_namespace": null, "vm_host_id": null, "vm_binding_kind": null,
            "legacy_vm_placement_required": false
        }),
        "agent_instance_volume_leases" => json!({
            "selection_origin": "historical_unproven", "selected_revision_id": null,
            "selected_release_id": null, "selected_release_agent_id": null,
            "selected_project_id": null, "slot_key": null, "access_mode": null,
            "guest_path": null, "slot_required": null, "minimum_capacity_bytes": null,
            "release_contract_hash": null, "named_binding_revision_id": null
        }),
        "agent_instance_state_volumes" => json!({
            "creation_seal_id": null, "creation_operation_id": null,
            "ready_retention_closure_id": null, "ready_retention_reopen_id": null
        }),
        "recipe_execution_transitions" => json!({
            "partial_retention_receipt_id": null, "ready_retention_fact_id": null
        }),
        "agent_instance_removal_requests" if public142 => {
            json!({"prepared_resume_command_id": null})
        }
        _ => json!({}),
    };
    match fields {
        Value::Object(map) => map,
        _ => unreachable!("literal default object"),
    }
}

fn lease_values(selection: Option<&Selection>) -> Map<String, Value> {
    selection.map_or_else(
        || defaults("agent_instance_volume_leases", false),
        |selected| selected.fields().clone(),
    )
}

fn rows<'a>(snapshot: &'a Snapshot, name: &str) -> Result<BTreeMap<Uuid, &'a Row>, sqlx::Error> {
    let table = snapshot
        .tables()
        .get(name)
        .ok_or_else(|| sqlx::Error::Protocol("required BEFORE/AFTER table missing".into()))?;
    require(
        table.columns.iter().filter(|c| c.name == "id").count() == 1
            && table
                .columns
                .iter()
                .any(|c| c.name == "id" && c.type_oid == 2950 && c.type_name == "uuid"),
        "source-defined UUID primary-key column changed",
    )?;
    let mut indexed = BTreeMap::new();
    for row in &table.rows {
        let id = uuid(row, "id")?;
        require(
            indexed.insert(id, row).is_none(),
            "duplicate source-defined primary-key identity",
        )?;
    }
    Ok(indexed)
}

fn field<'a>(row: &'a Row, name: &str) -> Result<&'a Value, sqlx::Error> {
    row.json
        .as_object()
        .and_then(|fields| fields.get(name))
        .ok_or_else(|| sqlx::Error::Protocol(format!("missing or malformed row field {name}")))
}

fn optional_uuid(row: &Row, name: &str) -> Result<Option<Uuid>, sqlx::Error> {
    let value = field(row, name)?;
    if value.is_null() {
        return Ok(None);
    }
    let text = value
        .as_str()
        .ok_or_else(|| sqlx::Error::Protocol("UUID field is not text or NULL".into()))?;
    let id = Uuid::parse_str(text).map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
    require(id.to_string() == text, "UUID field is not canonical")?;
    Ok(Some(id))
}

fn uuid(row: &Row, name: &str) -> Result<Uuid, sqlx::Error> {
    optional_uuid(row, name)?
        .ok_or_else(|| sqlx::Error::Protocol("required UUID is SQL NULL".into()))
}

fn string<'a>(row: &'a Row, name: &str) -> Result<&'a str, sqlx::Error> {
    field(row, name)?
        .as_str()
        .ok_or_else(|| sqlx::Error::Protocol("required text field is malformed".into()))
}

fn require(condition: bool, message: &str) -> Result<(), sqlx::Error> {
    if condition {
        Ok(())
    } else {
        Err(sqlx::Error::Protocol(message.into()))
    }
}
