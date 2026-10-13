//! Lossless graph multisets, with substitutions confined to typed endpoints.
use super::{
    Record, Semantic, bindings, catalog, identity, json, objects::Objects, records, replace,
    semantic, source::Changes,
};
use std::collections::BTreeMap;

type Key = (String, String);
pub(super) struct Edge<'a> {
    pub record: Record<'a>,
    pub semantic: Semantic,
    pub object: Key,
    pub reference: Key,
    pub kind: String,
}

pub(super) fn shape(record: Record<'_>, shared: bool) {
    let raw = &record.value["raw"];
    let fields = if shared {
        vec![
            "dbid",
            "classid",
            "objid",
            "objsubid",
            "refclassid",
            "refobjid",
            "deptype",
        ]
    } else {
        vec![
            "classid",
            "objid",
            "objsubid",
            "refclassid",
            "refobjid",
            "refobjsubid",
            "deptype",
        ]
    };
    identity::keys(raw, &fields);
    let mut fields = vec!["raw", "object", "reference"];
    if !shared {
        fields.extend(["object_address", "reference_address"]);
    }
    identity::keys(record.value, &fields);
    for field in ["classid", "refclassid"] {
        catalog(&raw[field]);
    }
    for field in ["objid", "refobjid"] {
        assert_ne!(identity::oid(&raw[field]), "0");
    }
    bindings::subobject(raw, shared, false);
    bindings::subobject(raw, shared, true);
    if shared {
        identity::oid(&raw["dbid"]);
    }
    let kind = identity::text(&raw["deptype"]);
    assert!(
        if shared {
            matches!(kind, "o" | "a" | "i" | "r" | "t")
        } else {
            matches!(kind, "n" | "a" | "i" | "P" | "S" | "e" | "x")
        },
        "known dependency type"
    );
    for field in ["object", "reference"] {
        identity::object(&record.value[field]);
    }
}

pub(super) fn inventory<'a>(objects: &Objects<'a>, shared: bool) -> Vec<Edge<'a>> {
    let mut result = Vec::new();
    for record in records(
        objects.structure.snapshot,
        if shared {
            "shared_dependencies"
        } else {
            "dependencies"
        },
    ) {
        shape(record, shared);
        let raw = &record.value["raw"];
        if shared && raw["dbid"] == "0" {
            continue;
        }
        let mut normalized = semantic(record);
        if shared {
            assert_eq!(
                identity::oid(&raw["dbid"]),
                objects.database,
                "current database witness"
            );
            replace(&mut normalized, "raw.dbid", &json!("current_database"));
        }
        let object = endpoint(objects, record, shared, false, &mut normalized);
        let reference = endpoint(objects, record, shared, true, &mut normalized);
        if shared {
            assert_eq!(
                reference.0,
                if raw["deptype"] == "t" {
                    "1213"
                } else {
                    "1260"
                }
            );
        }
        result.push(Edge {
            record,
            semantic: normalized,
            object,
            reference,
            kind: identity::text(&raw["deptype"]).to_owned(),
        });
    }
    result
}

fn endpoint(
    objects: &Objects<'_>,
    record: Record<'_>,
    shared: bool,
    reference: bool,
    result: &mut Semantic,
) -> Key {
    let raw = &record.value["raw"];
    let (class_field, id_field, helper_field, address_field) = if reference {
        ("refclassid", "refobjid", "reference", "reference_address")
    } else {
        ("classid", "objid", "object", "object_address")
    };
    let class = catalog(&raw[class_field]);
    let id = identity::oid(&raw[id_field]);
    let sub = bindings::subobject(raw, shared, reference);
    let actual_address = (!shared).then_some(&record.value[address_field]);
    let (key, helper, address) =
        objects.endpoint(class, id, sub, &record.value[helper_field], actual_address);
    // Catalog IDs and signed subIDs remain literal, including column order.
    if objects
        .bindings
        .contains_key(&(class.to_owned(), id.to_owned(), sub))
    {
        replace(result, &format!("raw.{id_field}"), &json!(key));
        if helper != record.value[helper_field] {
            replace(result, helper_field, &helper);
        }
        if let Some(actual) = actual_address {
            let canonical = address.unwrap();
            if canonical != *actual {
                replace(result, address_field, &canonical);
            }
        }
    }
    (class.to_owned(), key)
}

pub(super) fn semantics(edges: &[Edge<'_>]) -> BTreeMap<Semantic, usize> {
    let mut result = BTreeMap::new();
    for edge in edges {
        *result.entry(edge.semantic.clone()).or_default() += 1;
    }
    result
}

pub(super) fn preserve(before: &[Edge<'_>], after: &[Edge<'_>], changes: &Changes, shared: bool) {
    let mut raw = BTreeMap::<&str, usize>::new();
    for edge in after {
        *raw.entry(edge.record.text).or_default() += 1;
    }
    let mut normalized = semantics(after);
    for edge in before {
        if !shared && changes.changed(edge) {
            continue;
        }
        if changes.rewritten(edge) {
            consume(
                &mut normalized,
                &edge.semantic,
                "rewritten child edge preserved completely",
            );
        } else {
            consume(
                &mut raw,
                &edge.record.text,
                "original dependency raw/text preservation",
            );
        }
    }
}

pub(super) fn consume<T: Ord>(counts: &mut BTreeMap<T, usize>, key: &T, message: &str) {
    let count = counts.get_mut(key).expect(message);
    assert!(*count > 0, "{message}");
    *count -= 1;
}

pub(super) fn admit_additions(
    before: &[Edge<'_>],
    after: &[Edge<'_>],
    original: &Objects<'_>,
    reference: &Objects<'_>,
    changes: &Changes,
    shared: bool,
) {
    let mut old = semantics(before);
    for edge in after {
        if let Some(count) = old.get_mut(&edge.semantic)
            && *count > 0
        {
            *count -= 1;
            continue;
        }
        assert!(
            new_endpoint(original, reference, &edge.object)
                || new_endpoint(original, reference, &edge.reference)
                || (!shared && changes.changed(edge))
                || (shared && changes.granted(edge)),
            "edge addition has no source-qualified endpoint/change"
        );
    }
}

fn new_endpoint(original: &Objects<'_>, reference: &Objects<'_>, key: &Key) -> bool {
    let present = |objects: &Objects<'_>| {
        objects
            .bindings
            .iter()
            .any(|((class, _, _), binding)| class == &key.0 && binding.key == key.1)
    };
    present(reference) && !present(original)
}
