//! Captured structure, lookup closure and complete attributes; physical
//! `pg_class` observations remain outside this oracle. Source admission
//! and the earlier row, routine, column and constraint assertions are required.
use super::catalog_table_columns::json_text;
use super::{catalog::Snapshot, catalog_constraints, catalog_table_columns};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[path = "catalog_structure/attributes.rs"]
mod attributes;
#[path = "catalog_structure/dependencies.rs"]
mod dependencies;
#[path = "catalog_structure/extension_metadata.rs"]
mod extension_metadata;
#[path = "catalog_structure/identity.rs"]
mod identity;
#[path = "catalog_structure/indexes.rs"]
mod indexes;
#[path = "catalog_structure/lookups.rs"]
mod lookups;
#[path = "catalog_structure/policies.rs"]
mod policies;
#[path = "catalog_structure/relations.rs"]
mod relations;
#[path = "catalog_structure/rewrite.rs"]
mod rewrite;
#[cfg(test)]
#[path = "catalog_structure/tests.rs"]
mod tests;
#[path = "catalog_structure/triggers.rs"]
mod triggers;
#[path = "catalog_structure/types.rs"]
mod types;

#[derive(Clone, Copy)]
struct Record<'a> {
    value: &'a Value,
    text: &'a str,
}

type Inventory<'a> = BTreeMap<String, Record<'a>>;
type Semantic = BTreeMap<String, String>;

struct Structure<'a> {
    snapshot: &'a Snapshot,
    relations: Inventory<'a>,
    classes: BTreeMap<String, String>,
    types: Inventory<'a>,
    type_ids: BTreeMap<String, String>,
}

impl<'a> Structure<'a> {
    fn new(snapshot: &'a Snapshot) -> Self {
        let relations = relations::inventory(snapshot);
        let classes = identity::ids(&relations, "oid");
        let types = types::inventory(snapshot);
        let type_ids = identity::ids(&types, "oid");
        let result = Self {
            snapshot,
            relations,
            classes,
            types,
            type_ids,
        };
        relations::validate(&result);
        types::validate(&result);
        result
    }
}

pub fn assert_structure(before: &Snapshot, after: &Snapshot, reference: &Snapshot) {
    assert_eq!(before.context(), after.context());
    let mut expected_context = after.context().json().clone();
    assert_ne!(
        expected_context["database"],
        reference.context().json()["database"]
    );
    expected_context["database"] = reference.context().json()["database"].clone();
    assert_eq!(&expected_context, reference.context().json());
    let old = Structure::new(before);
    let upgraded = Structure::new(after);
    let fresh = Structure::new(reference);
    let replaced_children = rewrite::qualify(&old, &upgraded, &fresh);
    let shapes = catalog_table_columns::structural_shapes();
    let added_indexes = catalog_constraints::structural_added_indexes(reference);
    let added = relations::admitted_additions(&old, &fresh, &shapes.new_tables, added_indexes);
    let removed = "public.one_active_instance_run_lease";
    assert!(old.relations.contains_key(removed));
    let expected: BTreeSet<_> = old
        .relations
        .keys()
        .filter(|key| key.as_str() != removed)
        .cloned()
        .chain(added.iter().cloned())
        .collect();
    assert_eq!(
        fresh.relations.keys().cloned().collect::<BTreeSet<_>>(),
        expected
    );
    assert_eq!(
        upgraded.relations.keys().cloned().collect::<BTreeSet<_>>(),
        expected
    );
    for key in old.relations.keys() {
        if key != removed {
            relations::preserve(
                key,
                &old,
                &upgraded,
                &shapes.added_columns,
                &replaced_children,
            );
        }
    }
    for key in &added {
        assert!(
            !old.classes
                .contains_key(identity::oid(&upgraded.relations[key].value["raw"]["oid"]))
        );
    }
    for key in &expected {
        assert_eq!(
            relations::normalized(&upgraded, key),
            relations::normalized(&fresh, key),
            "canonical relation {key}"
        );
    }
    indexes::compare(&old, &upgraded, &fresh, &added, removed, &replaced_children);
    types::compare(&old, &upgraded, &fresh, &shapes.new_tables);
    triggers::compare(&old, &upgraded, &fresh);
    policies::compare(&old, &upgraded, &fresh);
    lookups::compare(&old, &upgraded, &fresh);
    // A separately accepted rewrite oracle may supply its qualified child set.
    attributes::compare(&old, &upgraded, &fresh, &added, removed, &replaced_children);
    extension_metadata::compare(&old, &upgraded, &fresh);
    dependencies::compare(&old, &upgraded, &fresh, &replaced_children);
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

// All unmodified fields retain their lossless JSONB spelling. Typed substitutions
// below replace particular fields, rather than walking or rewriting arbitrary JSON.
fn semantic(record: Record<'_>) -> Semantic {
    let mut result = BTreeMap::new();
    for name in record.value.as_object().unwrap().keys() {
        let text = json_text::field(record.text, name);
        if name == "raw" {
            for field in record.value["raw"].as_object().unwrap().keys() {
                result.insert(
                    format!("raw.{field}"),
                    json_text::field(text, field).to_owned(),
                );
            }
        } else {
            result.insert(name.clone(), text.to_owned());
        }
    }
    result
}

fn replace(result: &mut Semantic, field: &str, value: &Value) {
    assert!(
        result.insert(field.to_owned(), value.to_string()).is_some(),
        "missing field {field}"
    );
}

fn exact(original: Record<'_>, current: Record<'_>, message: &str) {
    assert_eq!(original.text, current.text, "{message}");
    assert_eq!(original.value, current.value, "{message}");
}
