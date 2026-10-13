//! Partial record substitutions exercise comparison helpers, not snapshot authority.
use super::super::{Inventory, Record, semantic};
use super::{assert_closure, assert_shape, family_closure, namespace};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

#[test]
fn closure_rejects_extra_and_missing_records() {
    let value = json!({"raw": {"oid": "1976"}});
    let text = value.to_string();
    let rows = Inventory::from([(
        "int8_ops".into(),
        Record {
            value: &value,
            text: &text,
        },
    )]);
    assert_closure(&rows, &BTreeSet::from(["int8_ops".into()]));
    for wrong in [
        BTreeSet::new(),
        BTreeSet::from(["int8_ops".into(), "unreferenced".into()]),
    ] {
        assert!(std::panic::catch_unwind(|| assert_closure(&rows, &wrong)).is_err());
    }
}

#[test]
fn family_reference_must_resolve() {
    let class = json!({"raw": {"opcfamily": "1976"}});
    let class_text = class.to_string();
    let classes = Inventory::from([(
        "used".into(),
        Record {
            value: &class,
            text: &class_text,
        },
    )]);
    let family = json!({"raw": {"oid": "1976"}, "identity": {"schema": "pg_catalog"}});
    let family_text = family.to_string();
    let families = Inventory::from([(
        "integer_ops".into(),
        Record {
            value: &family,
            text: &family_text,
        },
    )]);
    assert_eq!(
        family_closure(&classes, &families),
        BTreeSet::from(["integer_ops".into()])
    );
    assert!(std::panic::catch_unwind(|| family_closure(&classes, &BTreeMap::new())).is_err());
}

#[test]
fn full_lookup_comparison_preserves_type_owner_and_flags() {
    let value = json!({"raw": {"opcintype": "20", "opckeytype": "0", "opcowner": "10", "opcnamespace": "11", "opcmethod": "403", "opcfamily": "1976", "opcdefault": true}, "owner": "postgres"});
    let text = value.to_string();
    let expected = semantic(Record {
        value: &value,
        text: &text,
    });
    assert_shape(&expected, &expected);
    for (field, wrong) in [
        ("opcintype", json!("25")),
        ("opckeytype", json!("20")),
        ("opcowner", json!("42")),
        ("opcnamespace", json!("2200")),
        ("opcmethod", json!("405")),
        ("opcfamily", json!("1994")),
        ("opcdefault", json!(false)),
    ] {
        let mut changed = value.clone();
        changed["raw"][field] = wrong;
        let changed_text = changed.to_string();
        let actual = semantic(Record {
            value: &changed,
            text: &changed_text,
        });
        assert!(std::panic::catch_unwind(|| assert_shape(&actual, &expected)).is_err());
    }
}

#[test]
fn namespace_qualification_requires_exact_builtin_identity_and_complete_public_capture() {
    let public = json!({"raw":{"oid":"2200", "nspname":"public"}});
    let capture = [public.clone()];
    namespace(&json!("11"), "pg_catalog", &capture);
    namespace(&json!("2200"), "public", &capture);
    for (raw, schema) in [
        ("2200", "pg_catalog"),
        ("99", "pg_catalog"),
        ("011", "pg_catalog"),
        ("11", "public"),
        ("11", "pg_toast"),
        ("11", "other"),
    ] {
        assert!(std::panic::catch_unwind(|| namespace(&json!(raw), schema, &capture)).is_err());
    }
    for incomplete in [
        vec![],
        vec![public.clone(), public],
        vec![json!({"raw":{"oid":"11", "nspname":"pg_catalog"}})],
    ] {
        for (raw, schema) in [("11", "pg_catalog"), ("2200", "public")] {
            assert!(
                std::panic::catch_unwind(|| namespace(&json!(raw), schema, &incomplete)).is_err()
            );
        }
    }
}
