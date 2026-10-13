//! BEFORE-derived counts, complete counter rows and paired ordinal coverage.
use super::super::{new_fields::Qualification, snapshots::Snapshot};
use super::{clocks::Clocks, rows};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};
use uuid::Uuid;

pub(super) struct Rule {
    pub(super) count: i64,
    pub(super) cursor: i64,
    pub(super) version: i64,
    pub(super) retention: String,
    pub(super) project: Uuid,
    pub(super) states: BTreeMap<String, usize>,
}

pub(super) fn rules(
    qualification: &Qualification<'_>,
) -> Result<BTreeMap<Uuid, Rule>, sqlx::Error> {
    let before = qualification.before();
    let scopes = rows::indexed(
        before,
        "application_event_scopes",
        &rows::SCOPE_FIELDS,
        &["scope_kind", "scope_id"],
    )?;
    let aggregates = rows::indexed(
        before,
        "application_aggregate_versions",
        &rows::AGGREGATE_FIELDS,
        &["scope_kind", "scope_id", "aggregate_type", "aggregate_id"],
    )?;
    let mut result: BTreeMap<Uuid, Rule> = BTreeMap::new();
    for selection in qualification.selections().values() {
        let id = selection.instance();
        if let Entry::Vacant(entry) = result.entry(id) {
            let scope_key = vec!["agent_instance".into(), id.to_string()];
            let mut aggregate_key = scope_key.clone();
            aggregate_key.extend(["agent_instance".into(), id.to_string()]);
            let scope = scopes
                .get(&scope_key)
                .ok_or_else(|| sqlx::Error::Protocol("qualifying BEFORE scope missing".into()))?;
            let aggregate = aggregates.get(&aggregate_key).ok_or_else(|| {
                sqlx::Error::Protocol("qualifying BEFORE aggregate missing".into())
            })?;
            let cursor = rows::integer(scope, "committed_cursor")?;
            let version = rows::integer(aggregate, "aggregate_version")?;
            rows::require(cursor >= 0 && version > 0, "invalid BEFORE counter bounds")?;
            entry.insert(Rule {
                count: 0,
                cursor,
                version,
                retention: rows::text(scope, "retention_interval")?.into(),
                project: selection.project(),
                states: BTreeMap::new(),
            });
        }
        let rule = result
            .get_mut(&id)
            .ok_or_else(|| sqlx::Error::Protocol("missing derived rule".into()))?;
        rows::require(
            rule.project == selection.project(),
            "BEFORE qualifying project mismatch",
        )?;
        rule.count = rule
            .count
            .checked_add(1)
            .ok_or_else(|| sqlx::Error::Protocol("qualifying count overflow".into()))?;
        *rule
            .states
            .entry(normalized(selection.old_state())?.into())
            .or_insert(0) += 1;
    }
    for (id, rule) in &result {
        rows::require(
            usize::try_from(rule.count).ok() == qualification.counts().get(id).copied(),
            "independent qualifying counts disagree",
        )?;
        rows::require(
            rule.cursor.checked_add(rule.count).is_some()
                && rule.version.checked_add(rule.count).is_some(),
            "BEFORE counter delta would overflow",
        )?;
    }
    rows::require(
        result.len() == qualification.counts().len(),
        "extra qualifying count scope",
    )?;
    Ok(result)
}

fn normalized(state: &str) -> Result<&'static str, sqlx::Error> {
    match state {
        "active" => Ok("active"),
        "released" => Ok("succeeded"),
        "recovery_required" => Ok("paused"),
        _ => Err(sqlx::Error::Protocol(
            "unsupported BEFORE lease state".into(),
        )),
    }
}

pub(super) fn assert_after(
    before: &Snapshot,
    after: &Snapshot,
    rules: &BTreeMap<Uuid, Rule>,
    clocks: &Clocks,
) -> Result<(), sqlx::Error> {
    for (table, fields, keys) in [
        (
            "application_event_scopes",
            rows::SCOPE_FIELDS.as_slice(),
            &["scope_kind", "scope_id"][..],
        ),
        (
            "application_aggregate_versions",
            rows::AGGREGATE_FIELDS.as_slice(),
            &["scope_kind", "scope_id", "aggregate_type", "aggregate_id"][..],
        ),
    ] {
        let original = rows::indexed(before, table, fields, keys)?;
        let upgraded = rows::indexed(after, table, fields, keys)?;
        rows::require(
            original.keys().eq(upgraded.keys())
                && before.tables()[table].columns == after.tables()[table].columns,
            "counter family identities or columns changed",
        )?;
        for (key, row) in original {
            let id = rows::uuid(row, "scope_id")?;
            let rule = rules.get(&id).filter(|_| {
                key[0] == "agent_instance"
                    && (table == "application_event_scopes"
                        || (key[2] == "agent_instance" && key[3] == id.to_string()))
            });
            let actual = upgraded[&key];
            if let Some(rule) = rule {
                let mut expected = row.json.clone();
                if table == "application_event_scopes" {
                    expected["committed_cursor"] = json!(rule.cursor + rule.count);
                    expected["updated_at"] = clocks.clock.clone();
                } else {
                    expected["aggregate_version"] = json!(rule.version + rule.count);
                }
                rows::require(
                    actual.json == expected,
                    "counter changed beyond exact BEFORE count/clock delta",
                )?;
            } else {
                rows::require(
                    actual == row,
                    "unaffected complete JSONB/text counter row changed",
                )?;
            }
        }
    }
    Ok(())
}

pub(super) fn assert_coverage(
    rules: &BTreeMap<Uuid, Rule>,
    ordinals: &BTreeSet<(Uuid, i64)>,
    states: &BTreeMap<(Uuid, String), usize>,
) -> Result<(), sqlx::Error> {
    let expected_ordinals: BTreeSet<_> = rules
        .iter()
        .flat_map(|(id, rule)| (1..=rule.count).map(|n| (*id, n)))
        .collect();
    let expected_states: BTreeMap<_, _> = rules
        .iter()
        .flat_map(|(id, rule)| {
            rule.states
                .iter()
                .map(|(state, count)| ((*id, state.clone()), *count))
        })
        .collect();
    rows::require(
        *ordinals == expected_ordinals && *states == expected_states,
        "new events do not cover paired contiguous ordinals and BEFORE state multiset",
    )
}
