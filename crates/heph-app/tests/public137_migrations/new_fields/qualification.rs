//! Independent migration108 JOIN/WHERE evaluation over sealed BEFORE107 rows.
use super::super::snapshots::{Phase, Row, Snapshot};
use super::{field, optional_uuid, require, rows, string, uuid};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

/// A qualifying lease's independently selected values and event correlation.
#[derive(Debug)]
pub struct Selection {
    fields: Map<String, Value>,
    instance: Uuid,
    project: Uuid,
    old_state: String,
}

impl Selection {
    /// Complete twelve added values selected from BEFORE, never AFTER origins.
    #[must_use]
    pub const fn fields(&self) -> &Map<String, Value> {
        &self.fields
    }
    /// Actual BEFORE instance/scope/aggregate identity.
    #[must_use]
    pub const fn instance(&self) -> Uuid {
        self.instance
    }
    /// Actual BEFORE project identity for the future related-ID oracle.
    #[must_use]
    pub const fn project(&self) -> Uuid {
        self.project
    }
    /// Actual unchanged BEFORE lease state, before event normalization.
    #[must_use]
    pub fn old_state(&self) -> &str {
        &self.old_state
    }
}

/// Sealed qualification evidence retaining the real BEFORE snapshot lifetime.
#[derive(Debug)]
pub struct Qualification<'a> {
    before: &'a Snapshot,
    selections: BTreeMap<Uuid, Selection>,
    counts: BTreeMap<Uuid, usize>,
}

impl Qualification<'_> {
    /// Actual sealed BEFORE custody; no caller-supplied labels or mutable access.
    #[must_use]
    pub const fn before(&self) -> &Snapshot {
        self.before
    }
    /// Exact lease-ID keyed qualifying selections.
    #[must_use]
    pub const fn selections(&self) -> &BTreeMap<Uuid, Selection> {
        &self.selections
    }
    /// Independent qualifying counts per instance/scope/aggregate.
    #[must_use]
    pub const fn counts(&self) -> &BTreeMap<Uuid, usize> {
        &self.counts
    }
}

pub(super) fn derive(before: &Snapshot) -> Result<Qualification<'_>, sqlx::Error> {
    require(
        before.phase() == Phase::Public107,
        "qualification needs BEFORE107",
    )?;
    let leases = rows(before, "agent_instance_volume_leases")?;
    let runs = rows(before, "runs")?;
    let instances = rows(before, "agent_instances")?;
    let revisions = rows(before, "agent_instance_revisions")?;
    let agents = rows(before, "release_agents")?;
    let volumes = rows(before, "agent_instance_state_volumes")?;
    ids(&leases, &["volume_id", "instance_id"], &["run_id"])?;
    ids(
        &runs,
        &[],
        &[
            "instance_id",
            "instance_revision_id",
            "release_id",
            "release_agent_id",
        ],
    )?;
    ids(&instances, &["project_id"], &["state_volume_id"])?;
    ids(&revisions, &["instance_id", "release_agent_id"], &[])?;
    ids(&agents, &["release_id"], &[])?;
    ids(&volumes, &["project_id"], &["instance_id"])?;
    predicates(&leases, &instances, &agents)?;
    let mut selections = BTreeMap::new();
    let mut counts = BTreeMap::new();
    for (id, lease) in leases {
        let Some(run) = optional_uuid(lease, "run_id")?.and_then(|id| runs.get(&id)) else {
            continue;
        };
        let Some(instance) = optional_uuid(run, "instance_id")?.and_then(|id| instances.get(&id))
        else {
            continue;
        };
        let Some(revision) =
            optional_uuid(run, "instance_revision_id")?.and_then(|id| revisions.get(&id))
        else {
            continue;
        };
        let Some(agent) = optional_uuid(run, "release_agent_id")?.and_then(|id| agents.get(&id))
        else {
            continue;
        };
        let Some(volume) =
            optional_uuid(instance, "state_volume_id")?.and_then(|id| volumes.get(&id))
        else {
            continue;
        };
        if matches(lease, run, instance, revision, agent, volume)? {
            let selected = selection(lease, run, instance, agent, volume)?;
            *counts.entry(selected.instance()).or_insert(0) += 1;
            selections.insert(id, selected);
        }
    }
    Ok(Qualification {
        before,
        selections,
        counts,
    })
}

fn ids(
    rows: &BTreeMap<Uuid, &Row>,
    required: &[&str],
    nullable: &[&str],
) -> Result<(), sqlx::Error> {
    for row in rows.values() {
        for name in required {
            uuid(row, name)?;
        }
        for name in nullable {
            optional_uuid(row, name)?;
        }
    }
    Ok(())
}

fn predicates(
    leases: &BTreeMap<Uuid, &Row>,
    instances: &BTreeMap<Uuid, &Row>,
    agents: &BTreeMap<Uuid, &Row>,
) -> Result<(), sqlx::Error> {
    for lease in leases.values() {
        require(
            matches!(
                string(lease, "state")?,
                "active" | "released" | "recovery_required"
            ),
            "unknown BEFORE lease state",
        )?;
    }
    for instance in instances.values() {
        require(
            matches!(string(instance, "volume_mode")?, "legacy" | "named"),
            "malformed BEFORE volume mode",
        )?;
    }
    for agent in agents.values() {
        require(
            field(agent, "requires_state")?.as_bool().is_some(),
            "requires_state is not boolean",
        )?;
        hash_length(agent)?;
        empty_slots(agent)?;
    }
    Ok(())
}

fn matches(
    lease: &Row,
    run: &Row,
    instance: &Row,
    revision: &Row,
    agent: &Row,
    volume: &Row,
) -> Result<bool, sqlx::Error> {
    let instance_id = uuid(instance, "id")?;
    let agent_id = uuid(agent, "id")?;
    // Nullable SQL equality never makes NULL equal NULL. Exact primary-key
    // lookups above already enforce every remaining ON/WHERE identity equality.
    Ok(uuid(revision, "instance_id")? == instance_id
        && uuid(revision, "release_agent_id")? == agent_id
        && Some(uuid(agent, "release_id")?) == optional_uuid(run, "release_id")?
        && optional_uuid(volume, "instance_id")? == Some(instance_id)
        && uuid(volume, "project_id")? == uuid(instance, "project_id")?
        && uuid(lease, "instance_id")? == instance_id
        && uuid(lease, "volume_id")? == uuid(volume, "id")?
        && string(instance, "volume_mode")? == "legacy"
        && field(agent, "requires_state")?.as_bool() == Some(true)
        && hash_length(agent)? == 32
        && empty_slots(agent)?)
}

fn hash_length(agent: &Row) -> Result<usize, sqlx::Error> {
    let hash = string(agent, "runtime_contract_hash")?;
    let encoded = hash
        .strip_prefix("\\x")
        .ok_or_else(|| sqlx::Error::Protocol("bytea hash lacks PostgreSQL hex prefix".into()))?;
    require(
        encoded.len().is_multiple_of(2)
            && encoded
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "malformed BEFORE bytea hash",
    )?;
    Ok(encoded.len() / 2)
}

fn empty_slots(agent: &Row) -> Result<bool, sqlx::Error> {
    let contract = field(agent, "runtime_contract")?;
    // jsonb -> text returns SQL NULL for absent object keys and nonobjects.
    // COALESCE then supplies []. A present JSON null is not SQL NULL and
    // jsonb_array_length must fail, as do present scalar/object values.
    let slots = contract
        .as_object()
        .and_then(|object| object.get("volume_slots"));
    match slots {
        None => Ok(true),
        Some(Value::Array(slots)) => Ok(slots.is_empty()),
        Some(_) => Err(sqlx::Error::Protocol(
            "volume_slots is JSON null or nonarray".into(),
        )),
    }
}

fn selection(
    lease: &Row,
    run: &Row,
    instance: &Row,
    agent: &Row,
    volume: &Row,
) -> Result<Selection, sqlx::Error> {
    let fields = json!({
        "selection_origin": "legacy_origin",
        "selected_revision_id": field(run, "instance_revision_id")?,
        "selected_release_id": field(run, "release_id")?,
        "selected_release_agent_id": field(run, "release_agent_id")?,
        "selected_project_id": field(volume, "project_id")?,
        "slot_key": "state", "access_mode": "read_write",
        "guest_path": "/var/lib/hephaestus", "slot_required": true,
        "minimum_capacity_bytes": 1,
        "release_contract_hash": field(agent, "runtime_contract_hash")?,
        "named_binding_revision_id": null
    });
    let Value::Object(fields) = fields else {
        unreachable!("literal selection object")
    };
    Ok(Selection {
        fields,
        instance: uuid(instance, "id")?,
        project: uuid(instance, "project_id")?,
        old_state: string(lease, "state")?.into(),
    })
}
