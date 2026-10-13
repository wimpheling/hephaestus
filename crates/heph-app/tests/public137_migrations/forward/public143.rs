//! Exact142→143 identities; additions must be the independently source-admitted terminal graph.
use super::{
    Record, Snapshot, catalog_routines, consume, context, counts, exact, fields, indexed, records,
};
use serde_json::Value;
use std::collections::BTreeSet;
#[path = "public143/graph.rs"]
mod graph;
#[path = "public143/transport.rs"]
mod transport;
type Endpoint = (String, String, i64);
const TABLE: &str = "public.recipe_prepared_instance_terminal_operations143";
pub(super) fn assert143(before: &Snapshot, after: &Snapshot) {
    context(before, after);
    let mut added: BTreeSet<Endpoint> = catalog_routines::assert_forward143(before, after)
        .into_iter()
        .map(|id| ("1255".to_owned(), id, 0))
        .collect();
    routine_text(before, after);
    graph::relations(before, after, &mut added);
    graph::types(before, after, &mut added);
    graph::columns(before, after, &mut added);
    graph::indexes(before, after);
    transport::constraints(before, after, &mut added);
    transport::triggers(before, after, &mut added);
    transport::defaults(before, after, &mut added);
    for (domain, original) in before.domains() {
        match domain.as_str() {
            "routines" | "relations" | "types" | "columns" | "indexes" | "constraints"
            | "triggers" | "defaults" => (),
            "dependencies" | "shared_dependencies" => edges(before, after, domain, &added),
            _ => assert_eq!(
                original,
                &after.domains()[domain],
                "unchanged142→143 {domain}"
            ),
        }
    }
}
fn routine_text(before: &Snapshot, after: &Snapshot) {
    let current = indexed(after, "routines");
    let mut replaced = 0;
    for (id, original) in indexed(before, "routines") {
        let following = current[id];
        if catalog_routines::replaces143(original.value["raw"]["proname"].as_str().unwrap()) {
            let mut expected = fields(original);
            let actual = fields(following);
            for field in ["raw.prosrc", "definition"] {
                assert!(expected.insert(field.to_owned(), actual[field]).is_some());
            }
            assert_eq!(
                expected, actual,
                "143 preserves every old routine text field except exact source bodies"
            );
            replaced += 1;
        } else {
            exact(original, following);
        }
    }
    assert_eq!(replaced, 11);
}
fn endpoint(raw: &Value, reference: bool) -> Endpoint {
    let (class, id, sub) = if reference {
        ("refclassid", "refobjid", "refobjsubid")
    } else {
        ("classid", "objid", "objsubid")
    };
    (
        raw[class].as_str().unwrap().to_owned(),
        raw[id].as_str().unwrap().to_owned(),
        raw.get(sub).map_or(0, |v| v.as_i64().unwrap()),
    )
}
fn edges(before: &Snapshot, after: &Snapshot, domain: &str, added: &BTreeSet<Endpoint>) {
    let current = records(after, domain);
    let mut remaining = counts(&current);
    for original in records(before, domain) {
        consume(&mut remaining, original.text);
    }
    for record in current {
        let count = remaining.get_mut(record.text).unwrap();
        if *count == 0 {
            continue;
        }
        *count -= 1;
        let raw = &record.value["raw"];
        assert!(
            added.contains(&endpoint(raw, false))
                || (domain == "dependencies" && added.contains(&endpoint(raw, true))),
            "143 new edge must touch a positively qualified typed source-added endpoint"
        );
    }
}
fn additions<'a>(before: &Snapshot, after: &'a Snapshot, domain: &str) -> Vec<Record<'a>> {
    let old = indexed(before, domain);
    let current = indexed(after, domain);
    for (id, original) in &old {
        exact(*original, current[id]);
    }
    current
        .into_iter()
        .filter(|(id, _)| !old.contains_key(id))
        .map(|(_, r)| r)
        .collect()
}
fn relation_ids(after: &Snapshot) -> BTreeSet<String> {
    let graph = graph::declared_relations(after);
    graph
        .into_iter()
        .map(|r| r.value["raw"]["oid"].as_str().unwrap().to_owned())
        .collect()
}
