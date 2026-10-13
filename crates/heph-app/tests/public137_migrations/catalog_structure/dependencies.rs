//! Complete local graphs and literal global shared-edge multisets. The caller
//! qualifies journals/native inputs and passes all earlier source-bound oracles.
use super::{Record, Semantic, Structure, identity, records, replace, semantic};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[path = "dependencies/bindings.rs"]
mod bindings;
#[path = "dependencies/edges.rs"]
mod edges;
#[path = "dependencies/globals.rs"]
mod globals;
#[path = "dependencies/objects.rs"]
mod objects;
#[path = "dependencies/source.rs"]
mod source;
#[cfg(test)]
#[path = "dependencies/tests.rs"]
mod tests;

pub(super) fn compare(
    old: &Structure<'_>,
    upgraded: &Structure<'_>,
    fresh: &Structure<'_>,
    rewritten: &BTreeSet<String>,
) {
    let original = objects::Objects::new(old);
    let current = objects::Objects::new(upgraded);
    let reference = objects::Objects::new(fresh);
    let changes = source::Changes::new(&original, &current, &reference, rewritten);
    for shared in [false, true] {
        let before = edges::inventory(&original, shared);
        let after = edges::inventory(&current, shared);
        let canonical = edges::inventory(&reference, shared);
        assert_eq!(
            edges::semantics(&after),
            edges::semantics(&canonical),
            "complete canonical dependency multiset"
        );
        edges::preserve(&before, &after, &changes, shared);
        edges::admit_additions(&before, &after, &original, &reference, &changes, shared);
    }
    globals::compare(old, upgraded, fresh);
}

fn signed(value: &Value) -> i32 {
    i32::try_from(value.as_i64().expect("signed dependency subobject")).unwrap()
}

fn catalog(value: &Value) -> &str {
    let id = identity::oid(value);
    assert!(
        id.parse::<u32>().unwrap() > 0 && id.parse::<u32>().unwrap() < 16384,
        "literal native catalog class OID"
    );
    id
}

fn address(value: &Value, helper: &Value) {
    identity::keys(value, &["type", "object_names", "object_args"]);
    assert_eq!(value["type"], helper["type"]);
    for field in ["object_names", "object_args"] {
        assert!(
            value[field]
                .as_array()
                .unwrap()
                .iter()
                .all(Value::is_string)
        );
    }
    assert!(!value["object_names"].as_array().unwrap().is_empty());
}
