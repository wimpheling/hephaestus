//! Substitutions exercise pure helpers, not fabricated positive snapshots.
use super::{Record, bindings, catalog, edges, globals, json, semantic, signed};
use serde_json::Value;
use std::collections::BTreeMap;

fn helper() -> Value {
    json!({"type":"table column","schema":"public","name":"runs","identity":"public.runs.id"})
}

#[test]
#[should_panic(expected = "captured typed endpoint witness")]
fn mismatched_object_identity_denied() {
    let binding = bindings::Binding::new(
        "public.runs#1".into(),
        helper(),
        &["public".into(), "runs".into(), "id".into()],
    );
    let mut wrong = helper();
    wrong["identity"] = json!("public.runs.actor_id");
    binding.check(&wrong, None);
}

#[test]
#[should_panic(expected = "complete endpoint address witness")]
fn altered_address_arguments_denied() {
    let binding = bindings::Binding::new(
        "public.runs#1".into(),
        helper(),
        &["public".into(), "runs".into(), "id".into()],
    );
    let mut wrong = binding.address.clone().unwrap();
    wrong["object_args"] = json!(["pg_catalog.uuid"]);
    binding.check(&helper(), Some(&wrong));
}

#[test]
#[should_panic(expected = "known native endpoint catalog")]
fn unknown_catalog_denied() {
    bindings::raw_endpoint("9999", "10", 0, &helper(), None);
}

#[test]
#[should_panic(expected = "nonrelation subobject")]
fn function_column_denied() {
    bindings::raw_endpoint("1255", "10", 1, &helper(), None);
}

#[test]
#[should_panic(expected = "TryFromIntError")]
fn overflowing_signed_subobject_denied() {
    signed(&json!(2_147_483_648_i64));
}

#[test]
#[should_panic(expected = "literal native catalog class OID")]
fn dynamic_catalog_class_denied() {
    catalog(&json!("16384"));
}

#[test]
#[should_panic(expected = "left: String(\"other\")")]
fn wrong_role_witness_denied() {
    bindings::role(
        &json!({"type":"role","schema":null,"name":"other","identity":"other"}),
        &json!({"oid":"10","rolname":"postgres"}),
    );
}

#[test]
fn complete_global_bytes_and_multiplicity_are_kept() {
    let value = json!({"raw":{"objid":"99"},"object":{"identity":"peer"}});
    let first = r#"{"raw":{"objid":"99"},"object":{"identity":"peer"}}"#;
    let other = r#"{"raw":{"objid":"99"},"object":{"identity":"changed"}}"#;
    let other_value: Value = serde_json::from_str(other).unwrap();
    let mut counts = BTreeMap::new();
    globals::count(
        &mut counts,
        Record {
            value: &value,
            text: first,
        },
    );
    globals::count(
        &mut counts,
        Record {
            value: &value,
            text: first,
        },
    );
    globals::count(
        &mut counts,
        Record {
            value: &other_value,
            text: other,
        },
    );
    assert_eq!(counts[first], 2);
    assert_eq!(counts[other], 1);
    assert_eq!(counts.len(), 2);
}

#[test]
#[should_panic(expected = "missing original edge")]
fn lost_duplicate_edge_denied() {
    let mut counts = BTreeMap::from([("edge", 1)]);
    edges::consume(&mut counts, &"edge", "missing original edge");
    edges::consume(&mut counts, &"edge", "missing original edge");
}

#[test]
fn signed_column_and_dependency_flags_remain_exact() {
    let text = r#"{"raw":{"objsubid":-1,"refobjsubid":2,"deptype":"a"},"object":{"identity":"left"},"reference":{"identity":"right"}}"#;
    let value: Value = serde_json::from_str(text).unwrap();
    let fields = semantic(Record {
        value: &value,
        text,
    });
    assert_eq!(fields["raw.objsubid"], "-1");
    assert_eq!(fields["raw.refobjsubid"], "2");
    assert_eq!(fields["raw.deptype"], r#""a""#);
    assert_eq!(fields["reference"], r#"{"identity":"right"}"#);
}

#[test]
#[should_panic(expected = "uncaptured nonnative local endpoint")]
fn missing_local_witness_denied() {
    bindings::raw_endpoint("1255", "99999", 0, &helper(), None);
}

#[test]
#[should_panic(expected = "known dependency type")]
fn unknown_dependency_kind_denied() {
    let value = json!({"raw":{"classid":"1259","objid":"10","objsubid":0,"refclassid":"1247","refobjid":"23","refobjsubid":0,"deptype":"?"},"object":helper(),"reference":helper(),"object_address":{},"reference_address":{}});
    let text = value.to_string();
    edges::shape(
        Record {
            value: &value,
            text: &text,
        },
        false,
    );
}

#[test]
fn owner_or_acl_edge_change_cannot_equal_original_semantics() {
    let original = json!({"raw":{"deptype":"o","refobjid":"10"},"reference":{"type":"role","schema":null,"name":"postgres","identity":"postgres"}});
    let mut altered = original.clone();
    altered["raw"]["refobjid"] = json!("11");
    altered["raw"]["deptype"] = json!("a");
    let original_text = original.to_string();
    let altered_text = altered.to_string();
    assert_ne!(
        semantic(Record {
            value: &original,
            text: &original_text
        }),
        semantic(Record {
            value: &altered,
            text: &altered_text
        })
    );
}
