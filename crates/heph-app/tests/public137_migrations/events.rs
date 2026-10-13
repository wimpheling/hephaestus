//! Exact migration108 durable-event delta; no AFTER-derived expected payloads.
use super::{
    journal,
    new_fields::Qualification,
    snapshots::{Phase, Row, Snapshot},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

mod clocks;
mod counters;
mod rows;

// Fresh identifiers and visit position are validated against BEFORE evidence.
struct EventIdentity {
    id: Uuid,
    occurrence: Uuid,
    scope: Uuid,
    ordinal: i64,
}

/// Check full inherited rows and exact BEFORE-qualified event/counter appends.
///
/// # Errors
/// Rejects journal drift, changed prior rows, missing/extra appends, identity,
/// payload, ordinal, state-count, calendar, counter or owner-context mismatch.
/// Calendar computation owns one RR/read-only UTC transaction with awaited
/// commit/rollback; cancellation cannot promise a synchronous Drop-time join.
pub async fn assert_upgrade(
    pool: &PgPool,
    qualification: &Qualification<'_>,
    after: &Snapshot,
) -> Result<(), sqlx::Error> {
    let before = qualification.before();
    rows::require(
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
        "event oracle needs107→137..143",
    )?;
    // Internally validate these exact snapshots, rather than accept a journal
    // token from an unrelated AFTER snapshot with merely the same phase.
    let original = journal::validate(before, None)?;
    let upgraded = journal::validate(after, Some(&original))?;
    let installed108 = upgraded
        .installed108()
        .ok_or_else(|| sqlx::Error::Protocol("validated journal lacks108 clock".into()))?;
    let rules = counters::rules(qualification)?;
    let clocks = clocks::compute(pool, installed108, &rules).await?;
    let events = rows::appended(
        before,
        after,
        "application_events",
        &rows::EVENT_FIELDS,
        &["id"],
    )?;
    let outbox = rows::appended(
        before,
        after,
        "product_event_outbox",
        &rows::OUTBOX_FIELDS,
        &["event_id"],
    )?;
    rows::require(
        events.len() == qualification.selections().len() && events.keys().eq(outbox.keys()),
        "event/outbox append count or exact event-ID bijection differs from BEFORE qualifiers",
    )?;
    validate_events(before, &events, &outbox, &rules, &clocks)?;
    counters::assert_after(before, after, &rules, &clocks)?;
    // This fifth durable-event table has no declared migration108 changes.
    rows::unchanged(before, after, "product_event_dead_letters")
}

fn validate_events(
    before: &Snapshot,
    events: &BTreeMap<Vec<String>, &Row>,
    outbox: &BTreeMap<Vec<String>, &Row>,
    rules: &BTreeMap<Uuid, counters::Rule>,
    clocks: &clocks::Clocks,
) -> Result<(), sqlx::Error> {
    let mut reserved = old_identities(before)?;
    let mut ordinals = BTreeSet::new();
    let mut states = BTreeMap::new();
    for (key, event) in events {
        let id = rows::uuid(event, "id")?;
        let occurrence = rows::uuid(event, "occurrence_id")?;
        rows::fresh(id, &mut reserved)?;
        rows::fresh(occurrence, &mut reserved)?;
        let scope = rows::uuid(event, "scope_id")?;
        let rule = rules
            .get(&scope)
            .ok_or_else(|| sqlx::Error::Protocol("new event scope not BEFORE-qualified".into()))?;
        let ordinal = rows::integer(event, "cursor")?
            .checked_sub(rule.cursor)
            .filter(|n| *n > 0 && *n <= rule.count)
            .ok_or_else(|| {
                sqlx::Error::Protocol("event cursor outside BEFORE ordinal range".into())
            })?;
        rows::require(
            ordinals.insert((scope, ordinal)),
            "duplicate paired event ordinal",
        )?;
        let actual_state = rows::text(event, "safe_state")?;
        let state = rule
            .states
            .keys()
            .find(|state| state.as_str() == actual_state)
            .ok_or_else(|| {
                sqlx::Error::Protocol("event state absent from BEFORE normalized states".into())
            })?;
        *states.entry((scope, state.clone())).or_insert(0) += 1;
        assert_event(
            event,
            &EventIdentity {
                id,
                occurrence,
                scope,
                ordinal,
            },
            rule,
            clocks,
            state,
        )?;
        assert_outbox(outbox[key], id, &clocks.clock)?;
    }
    counters::assert_coverage(rules, &ordinals, &states)
}

fn assert_event(
    event: &Row,
    identity: &EventIdentity,
    rule: &counters::Rule,
    clocks: &clocks::Clocks,
    state: &str,
) -> Result<(), sqlx::Error> {
    let retained = clocks
        .retained
        .get(&identity.scope)
        .ok_or_else(|| sqlx::Error::Protocol("BEFORE calendar retention missing".into()))?;
    let expected = json!({
        "id": identity.id.to_string(), "occurrence_id": identity.occurrence.to_string(),
        "scope_kind": "agent_instance", "scope_id": identity.scope.to_string(),
        "cursor": rule.cursor + identity.ordinal, "aggregate_type": "agent_instance",
        "aggregate_id": identity.scope.to_string(), "aggregate_version": rule.version + identity.ordinal,
        "event_type": "agent_instance.changed", "schema_version": 1,
        "change_kind": "updated", "safe_state": state,
        "related_id_one": rule.project.to_string(), "related_id_two": null,
        "actor_type": "system", "actor_id": null, "request_id": null,
        "occurred_at": clocks.clock, "retained_until": retained
    });
    rows::require(
        event.json == expected,
        "complete19-field event differs from independent BEFORE oracle",
    )
}

fn assert_outbox(row: &Row, id: Uuid, clock: &Value) -> Result<(), sqlx::Error> {
    let expected = json!({
        "event_id": id.to_string(), "subject": "hephaestus.product.event.v1",
        "created_at": clock, "published_at": null, "dead_lettered_at": null,
        "attempts": 0, "last_error": null, "terminal_reason": null
    });
    rows::require(
        row.json == expected,
        "complete8-field outbox differs from independent event-ID/clock oracle",
    )
}

fn old_identities(before: &Snapshot) -> Result<BTreeSet<Uuid>, sqlx::Error> {
    let events = rows::indexed(before, "application_events", &rows::EVENT_FIELDS, &["id"])?;
    let mut reserved = BTreeSet::new();
    for row in events.values() {
        reserved.insert(rows::uuid(row, "id")?);
        // A prior request may legitimately reuse an occurrence across events.
        reserved.insert(rows::uuid(row, "occurrence_id")?);
    }
    Ok(reserved)
}
