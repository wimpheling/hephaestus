//! Exact same-database forward checkpoints; no fresh OID normalization occurs.
use super::{catalog::Snapshot, catalog_routines, catalog_table_columns::json_text};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy)]
struct Record<'a> {
    value: &'a Value,
    text: &'a str,
}

type Endpoints = BTreeSet<(String, String)>;

#[path = "forward/public142.rs"]
mod public142;
#[path = "forward/public143.rs"]
mod public143;
#[path = "forward/secret_acl.rs"]
mod secret_acl;
use secret_acl::{relations141, shared141};
pub(super) fn assert143(before: &Snapshot, after: &Snapshot) {
    public143::assert143(before, after);
}

pub(super) fn assert142(before: &Snapshot, after: &Snapshot) {
    public142::assert142(before, after);
}

pub(super) fn assert140(before: &Snapshot, after: &Snapshot) {
    context(before, after);
    let routines = catalog_routines::assert_forward140(before, after);
    let mut added: Endpoints = routines
        .into_iter()
        .map(|id| ("1255".to_owned(), id))
        .collect();
    named_additions(
        before,
        after,
        "triggers",
        "tgname",
        &[
            ("b_queued_invocation_cancel_inbox140", "command_inbox"),
            ("ordinary_queued_cancel_inbox_commit140", "command_inbox"),
            ("ordinary_queued_cancel_run_commit140", "runs"),
        ],
        "2620",
        &mut added,
    );
    named_additions(
        before,
        after,
        "constraints",
        "conname",
        &[
            ("ordinary_queued_cancel_inbox_commit140", "command_inbox"),
            ("ordinary_queued_cancel_run_commit140", "runs"),
        ],
        "2606",
        &mut added,
    );
    routine_text(before, after);
    for (domain, original) in before.domains() {
        match domain.as_str() {
            "routines" | "triggers" | "constraints" => (),
            "dependencies" | "shared_dependencies" => edges140(before, after, domain, &added),
            _ => assert_eq!(
                original,
                &after.domains()[domain],
                "unchanged139→140 {domain}"
            ),
        }
    }
}

pub(super) fn assert141(before: &Snapshot, after: &Snapshot) {
    context(before, after);
    for (domain, original) in before.domains() {
        match domain.as_str() {
            "relations" => relations141(before, after),
            "shared_dependencies" => shared141(before, after),
            _ => assert_eq!(
                original,
                &after.domains()[domain],
                "unchanged140→141 {domain}"
            ),
        }
    }
}

fn context(before: &Snapshot, after: &Snapshot) {
    assert_eq!(before.context(), after.context());
    assert!(before.domains().keys().eq(after.domains().keys()));
    assert_eq!(before.sequences(), after.sequences());
}

fn records<'a>(snapshot: &'a Snapshot, domain: &str) -> Vec<Record<'a>> {
    let capture = &snapshot.domains()[domain];
    let values = capture.json().as_array().unwrap();
    let texts = json_text::array_items(capture.text());
    assert_eq!(values.len(), texts.len());
    values
        .iter()
        .zip(texts)
        .map(|(value, text)| {
            assert_eq!(*value, serde_json::from_str::<Value>(text).unwrap());
            Record { value, text }
        })
        .collect()
}

fn indexed<'a>(snapshot: &'a Snapshot, domain: &str) -> BTreeMap<&'a str, Record<'a>> {
    let mut result = BTreeMap::new();
    for record in records(snapshot, domain) {
        let oid = record.value["raw"]["oid"].as_str().unwrap();
        assert!(result.insert(oid, record).is_none());
    }
    result
}

fn exact(original: Record<'_>, following: Record<'_>) {
    assert_eq!(original.value, following.value);
    assert_eq!(original.text, following.text);
}

fn fields(record: Record<'_>) -> BTreeMap<String, &str> {
    let mut result = BTreeMap::new();
    for key in record.value.as_object().unwrap().keys() {
        let text = json_text::field(record.text, key);
        if key == "raw" {
            for field in record.value["raw"].as_object().unwrap().keys() {
                result.insert(format!("raw.{field}"), json_text::field(text, field));
            }
        } else {
            result.insert(key.clone(), text);
        }
    }
    result
}

fn routine_text(before: &Snapshot, after: &Snapshot) {
    let current = indexed(after, "routines");
    for (oid, original) in indexed(before, "routines") {
        let following = current[oid];
        if matches!(
            original.value["raw"]["proname"].as_str().unwrap(),
            "invocation119_queued_cleanup_zero_history"
                | "assert_invocation119_queued_cleanup_transition"
                | "validate_invocation119_queued_cleanup"
                | "run_cleanup_wake_original118"
                | "run_cleanup_wake_closure118"
                | "invocation119_cancel_closure"
                | "invocation119_cancel_original"
                | "invocation119_queued_cleanup_events"
                | "invocation119_cleanup_only_matches"
                | "invocation119_remove_resume_matches"
                | "guard_remove_resume_admission118"
                | "load_existing_instance_closure118"
        ) {
            let mut expected = fields(original);
            let actual = fields(following);
            for field in ["raw.prosrc", "definition"] {
                assert!(expected.insert(field.to_owned(), actual[field]).is_some());
            }
            assert_eq!(
                expected, actual,
                "only sealed140 routine body/definition can change"
            );
        } else {
            exact(original, following);
        }
    }
}

fn named_additions(
    before: &Snapshot,
    after: &Snapshot,
    domain: &str,
    name_field: &str,
    declarations: &[(&str, &str)],
    class: &str,
    added: &mut Endpoints,
) {
    let old = indexed(before, domain);
    let current = indexed(after, domain);
    for (oid, record) in &old {
        exact(*record, current[oid]);
    }
    let additions: Vec<_> = current
        .iter()
        .filter(|(oid, _)| !old.contains_key(*oid))
        .collect();
    assert_eq!(
        additions.len(),
        declarations.len(),
        "finite140 {domain} additions"
    );
    for (name, table) in declarations {
        let matches: Vec<_> = additions
            .iter()
            .filter(|(_, record)| record.value["raw"][name_field] == *name)
            .collect();
        assert_eq!(matches.len(), 1, "exact source140 named object");
        let record = matches[0].1;
        let object_oid = record.value["raw"]["oid"].as_str().unwrap();
        assert_eq!(record.value["relation"]["schema"], "public");
        assert_eq!(record.value["relation"]["name"], *table);
        if domain == "constraints" {
            assert_eq!(record.value["raw"]["contype"], "t");
        }
        assert!(added.insert((class.to_owned(), object_oid.to_owned())));
    }
}

fn counts<'a>(records: &[Record<'a>]) -> BTreeMap<&'a str, usize> {
    let mut result = BTreeMap::new();
    for record in records {
        *result.entry(record.text).or_default() += 1;
    }
    result
}

fn consume(counts: &mut BTreeMap<&str, usize>, text: &str) {
    let count = counts
        .get_mut(text)
        .expect("every original forward edge survives raw/text exact");
    assert!(*count > 0);
    *count -= 1;
}

fn edges140(before: &Snapshot, after: &Snapshot, domain: &str, added: &Endpoints) {
    let current = records(after, domain);
    let mut remaining = counts(&current);
    for record in records(before, domain) {
        consume(&mut remaining, record.text);
    }
    for record in current {
        let count = remaining.get_mut(record.text).unwrap();
        if *count == 0 {
            continue;
        }
        *count -= 1;
        let raw = &record.value["raw"];
        let object = (
            raw["classid"].as_str().unwrap().to_owned(),
            raw["objid"].as_str().unwrap().to_owned(),
        );
        let reference = (
            raw["refclassid"].as_str().unwrap().to_owned(),
            raw["refobjid"].as_str().unwrap().to_owned(),
        );
        assert!(
            added.contains(&object) || (domain == "dependencies" && added.contains(&reference)),
            "forward140 edge must touch exact source-added endpoint"
        );
    }
}
