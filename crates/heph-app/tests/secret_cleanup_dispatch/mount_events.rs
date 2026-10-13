//! Independent public0010 mount-trigger oracle for these fresh public107 cases.
//! No production serializer or SQL mutation is used to construct expected rows.
use serde_json::{Value, json};
use std::collections::BTreeSet;
use time::{Date, Duration, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};
use uuid::{Uuid, Variant};

pub(super) fn apply(
    before: &Value,
    after: &Value,
    expected: &mut Value,
    change: &str,
    state: &str,
    clock: &Value,
) {
    let added = fresh_row(
        &before["application_events"],
        &after["application_events"],
        "id",
    );
    let event = expected_event(before, expected, added, change, state, clock);
    let outbox = json!({
        "event_id": event["id"],
        "subject": "hephaestus.product.event.v1",
        "created_at": clock,
        "published_at": null,
        "dead_lettered_at": null,
        "attempts": 0,
        "last_error": null,
        "terminal_reason": null
    });
    expected["application_events"] = append(
        &before["application_events"],
        &after["application_events"],
        &event,
        "id",
    );
    expected["product_event_outbox"] = append(
        &before["product_event_outbox"],
        &after["product_event_outbox"],
        &outbox,
        "event_id",
    );
}

fn expected_event(
    before: &Value,
    expected: &Value,
    added: &Value,
    change: &str,
    state: &str,
    clock: &Value,
) -> Value {
    let run = &expected["secret_runtime_mounts"][0]["run_id"];
    let (project, repository) = relations(before, run);
    random_uuid(&added["id"], &before["application_events"]);
    random_uuid(&added["occurrence_id"], &before["application_events"]);
    assert_ne!(added["id"], added["occurrence_id"]);
    let occurred = timestamp(clock);
    let retained = timestamp(&added["retained_until"]);
    // These fixtures use the default30-day scope retention with no pruning.
    // Validate calendar days and local clock, allowing the session's DST offset.
    assert_eq!(retained.date(), occurred.date() + Duration::days(30));
    assert_eq!(retained.time(), occurred.time());
    for previous in before["application_events"].as_array().unwrap() {
        assert!(timestamp(&previous["occurred_at"]) <= occurred);
    }
    json!({
        "id": added["id"],
        "occurrence_id": added["occurrence_id"],
        "scope_kind": "run",
        "scope_id": run,
        "cursor": next_version(before, run, "cursor"),
        "aggregate_type": "run",
        "aggregate_id": run,
        "aggregate_version": next_version(before, run, "aggregate_version"),
        "event_type": "run.changed",
        "schema_version": 1,
        "change_kind": change,
        "safe_state": state,
        "related_id_one": project,
        "related_id_two": repository,
        "actor_type": "system",
        "actor_id": null,
        "request_id": null,
        "occurred_at": clock,
        "retained_until": added["retained_until"]
    })
}

fn relations<'a>(before: &'a Value, run: &Value) -> (&'a Value, Value) {
    let runs: Vec<_> = before["runs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["id"] == *run)
        .collect();
    assert_eq!(runs.len(), 1);
    let instances: Vec<_> = before["agent_instances"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["id"] == runs[0]["instance_id"])
        .collect();
    assert_eq!(instances.len(), 1);
    let project = &instances[0]["project_id"];
    Uuid::parse_str(project.as_str().unwrap()).unwrap();
    let requests: Vec<_> = before["run_requests"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["run_id"] == *run)
        .collect();
    assert!(requests.len() <= 1);
    let repository = requests
        .first()
        .map_or(Value::Null, |row| row["repository_id"].clone());
    (project, repository)
}

fn next_version(before: &Value, run: &Value, field: &str) -> u64 {
    before["application_events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["scope_kind"] == "run" && row["scope_id"] == *run)
        .map(|row| {
            assert_eq!(row["aggregate_type"], "run");
            assert_eq!(row["aggregate_id"], *run);
            row[field].as_u64().unwrap()
        })
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .unwrap()
}

fn fresh_row<'a>(before: &Value, after: &'a Value, key: &str) -> &'a Value {
    let previous = before.as_array().unwrap();
    let current = after.as_array().unwrap();
    let ids: BTreeSet<_> = previous
        .iter()
        .map(|row| row[key].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), previous.len());
    assert_eq!(current.len(), previous.len() + 1);
    let new: Vec<_> = current
        .iter()
        .filter(|row| !ids.contains(row[key].as_str().unwrap()))
        .collect();
    assert_eq!(new.len(), 1);
    new[0]
}

fn random_uuid(value: &Value, previous: &Value) {
    let text = value.as_str().unwrap();
    let id = Uuid::parse_str(text).unwrap();
    assert_eq!(id.to_string(), text);
    assert_eq!(id.get_variant(), Variant::RFC4122);
    assert_eq!(id.get_version_num(), 4);
    for row in previous.as_array().unwrap() {
        assert_ne!(value, &row["id"]);
        assert_ne!(value, &row["occurrence_id"]);
    }
}

fn append(before: &Value, after: &Value, added: &Value, key: &str) -> Value {
    let old = before.as_array().unwrap();
    let actual = after.as_array().unwrap();
    assert_eq!(actual.len(), old.len() + 1);
    let mut previous = old.iter();
    let mut inserted = false;
    let mut expected = Vec::with_capacity(actual.len());
    for row in actual {
        if row[key] == added[key] {
            assert!(!inserted, "exactly one new row");
            inserted = true;
            assert_eq!(row, added);
            expected.push(added.clone());
        } else {
            let original = previous.next().unwrap();
            assert_eq!(row, original, "every old row and its order are retained");
            expected.push(original.clone());
        }
    }
    assert!(inserted);
    assert!(previous.next().is_none());
    Value::Array(expected)
}

fn timestamp(value: &Value) -> OffsetDateTime {
    let text = value.as_str().unwrap();
    let (date, clock) = text.split_once('T').unwrap();
    let date: Vec<_> = date.split('-').collect();
    assert_eq!(date.len(), 3);
    let date = Date::from_calendar_date(
        date[0].parse().unwrap(),
        Month::try_from(date[1].parse::<u8>().unwrap()).unwrap(),
        date[2].parse().unwrap(),
    )
    .unwrap();
    let (clock, offset) = clock.split_at(clock.len().checked_sub(6).unwrap());
    assert_eq!(&offset[3..4], ":");
    let sign: i8 = match &offset[..1] {
        "+" => 1,
        "-" => -1,
        _ => panic!("numeric time offset"),
    };
    let offset = UtcOffset::from_hms(
        sign * offset[1..3].parse::<i8>().unwrap(),
        sign * offset[4..6].parse::<i8>().unwrap(),
        0,
    )
    .unwrap();
    let (clock, fraction) = clock.split_once('.').unwrap_or((clock, ""));
    assert!(fraction.len() <= 6 && fraction.bytes().all(|byte| byte.is_ascii_digit()));
    let nanos = format!("{fraction:0<9}").parse().unwrap();
    let clock: Vec<_> = clock.split(':').collect();
    assert_eq!(clock.len(), 3);
    let clock = Time::from_hms_nano(
        clock[0].parse().unwrap(),
        clock[1].parse().unwrap(),
        clock[2].parse().unwrap(),
        nanos,
    )
    .unwrap();
    PrimitiveDateTime::new(date, clock).assume_offset(offset)
}
