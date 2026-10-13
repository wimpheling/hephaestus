//! Substitutions exercise complete comparisons without inventing catalog snapshots.
use super::super::{Record, exact, semantic};
use super::{assert_shape, number};
use serde_json::json;

#[test]
fn attribute_numbers_are_signed_nonzero_i16() {
    for valid in [-6, -1, 1, 32767] {
        assert_eq!(i64::from(number(&json!({"attnum": valid}))), valid);
    }
    for wrong in [0, -32769, 32768] {
        assert!(std::panic::catch_unwind(|| number(&json!({"attnum": wrong}))).is_err());
    }
}

#[test]
fn full_attribute_shape_retains_storage_type_acl_and_position() {
    let value = json!({"raw": {"atttypid": "20", "attstorage": "p", "attcompression": "", "attnum": -1, "attacl": null, "attstattarget": -1, "attoptions": null}});
    let text = value.to_string();
    let expected = semantic(Record {
        value: &value,
        text: &text,
    });
    for (field, wrong) in [
        ("atttypid", json!("25")),
        ("attstorage", json!("x")),
        ("attcompression", json!("p")),
        ("attnum", json!(1)),
        ("attacl", json!(["hephaestus_app=r/postgres"])),
        ("attstattarget", json!(100)),
        ("attoptions", json!(["n_distinct=1"])),
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
fn old_missing_value_text_is_lossless() {
    let text = r#"{"raw":{"attmissingval":9007199254740993}}"#;
    let changed = r#"{"raw":{"attmissingval":9007199254740992}}"#;
    let original = serde_json::from_str(text).unwrap();
    let current = serde_json::from_str(changed).unwrap();
    assert!(
        std::panic::catch_unwind(|| exact(
            Record {
                value: &original,
                text
            },
            Record {
                value: &current,
                text: changed
            },
            "old missing datum"
        ))
        .is_err()
    );
}

#[test]
fn quoted_public_identity_and_attribute_bindings_remain_complete() {
    // Isolated helper fragments exercise field retention, not Snapshot authority.
    let value = json!({
        "raw": {"attrelid": "90000", "attname": "coalesce", "attnum": 1, "atttypid": "20"},
        "relation": {"type": "index", "schema": "public", "name": "developer_pats_by_owner_creation_request", "identity": "public.developer_pats_by_owner_creation_request"},
        "column_identity": {"type": "index column", "schema": "public", "name": "developer_pats_by_owner_creation_request", "identity": "public.developer_pats_by_owner_creation_request.\"coalesce\""},
        "type": {"type": "type", "schema": "pg_catalog", "name": "int8", "identity": "bigint"}
    });
    let text = value.to_string();
    let expected = semantic(Record {
        value: &value,
        text: &text,
    });
    for (section, field, wrong) in [
        (
            "column_identity",
            "identity",
            json!("public.developer_pats_by_owner_creation_request.coalesce"),
        ),
        (
            "column_identity",
            "identity",
            json!("public.other.\"coalesce\""),
        ),
        ("column_identity", "schema", json!("pg_catalog")),
        ("column_identity", "name", json!("other")),
        ("column_identity", "type", json!("table column")),
        ("raw", "attname", json!("other")),
        ("raw", "attnum", json!(2)),
        ("raw", "atttypid", json!("25")),
        ("relation", "identity", json!("public.other")),
    ] {
        let mut changed = value.clone();
        changed[section][field] = wrong;
        let changed_text = changed.to_string();
        let actual = semantic(Record {
            value: &changed,
            text: &changed_text,
        });
        assert!(std::panic::catch_unwind(|| assert_shape(&actual, &expected)).is_err());
    }
}

#[test]
fn public_identity_json_spelling_is_not_normalized() {
    let text = r#"{"column_identity":{"identity":"public.i.\"coalesce\""}}"#;
    let changed = r#"{"column_identity":{"identity":"public.i.\"\u0063oalesce\""}}"#;
    let original = serde_json::from_str(text).unwrap();
    let current = serde_json::from_str(changed).unwrap();
    assert_eq!(original, current);
    let expected = semantic(Record {
        value: &original,
        text,
    });
    let actual = semantic(Record {
        value: &current,
        text: changed,
    });
    assert!(std::panic::catch_unwind(|| assert_shape(&actual, &expected)).is_err());
}
