use super::{Record, exact, identity, indexes, relations, replace, semantic, types};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::panic::catch_unwind;

fn object(name: &str) -> Value {
    json!({"type": "table", "schema": "public", "name": name, "identity": format!("public.{name}")})
}

#[test]
fn typed_links_accept_independent_oids_and_reject_wrong_identity_and_duplicate_oid() {
    let helper = object("declared");
    for id in ["17000", "29000"] {
        let value = json!({"raw": {"oid": id}, "identity": helper});
        let record = Record {
            value: &value,
            text: "",
        };
        let inventory = BTreeMap::from([("public.declared".to_owned(), record)]);
        let ids = identity::ids(&inventory, "oid");
        assert_eq!(
            identity::captured_link(&inventory, &ids, &json!(id), &helper),
            "public.declared"
        );
        assert!(
            catch_unwind(|| identity::captured_link(
                &inventory,
                &ids,
                &json!(id),
                &object("other")
            ))
            .is_err()
        );
        assert!(
            catch_unwind(|| identity::captured_link(&inventory, &ids, &json!("99999"), &helper))
                .is_err()
        );
        let duplicate =
            BTreeMap::from([("first".to_owned(), record), ("second".to_owned(), record)]);
        assert!(catch_unwind(|| identity::ids(&duplicate, "oid")).is_err());
    }
}

#[test]
fn finite_toast_identity_rejects_spoofed_name_or_schema() {
    let name = identity::generated_name("17000", false);
    let value = json!({"type":"toast table", "schema":"pg_toast", "name":name,
        "identity":format!("pg_toast.{name}")});
    let stable = identity::toast_identity(&value, &name, "toast(public.declared)");
    let other_name = identity::generated_name("29000", false);
    let other = json!({"type":"toast table", "schema":"pg_toast", "name":other_name,
        "identity":format!("pg_toast.{other_name}")});
    assert_eq!(
        stable,
        identity::toast_identity(&other, &other_name, "toast(public.declared)")
    );
    assert!(
        catch_unwind(|| identity::toast_identity(&value, &other_name, "toast(public.declared)"))
            .is_err()
    );
    let column = json!({"type":"toast table column", "schema":"pg_toast", "name":name,
        "identity":format!("pg_toast.{name}.chunk_id")});
    assert_eq!(
        identity::toast_column_identity(&column, &name, "chunk_id", "toast(public.declared)")["name"],
        "toast(public.declared)"
    );
    assert!(
        catch_unwind(|| identity::toast_column_identity(
            &column,
            &name,
            "chunk_seq",
            "toast(public.declared)"
        ))
        .is_err()
    );
    let mut spoofed = value;
    spoofed["schema"] = json!("public");
    assert!(
        catch_unwind(|| identity::toast_identity(&spoofed, &name, "toast(public.declared)"))
            .is_err()
    );
}

#[test]
fn identity_substitution_retains_security_flags_vectors_and_predicate_constants() {
    let original = json!({"raw":{"oid":"17000", "relowner":"10", "relacl":["postgres=arwdDxtm/postgres"],
        "relrowsecurity":true, "relforcerowsecurity":true, "indisvalid":true, "indisready":true,
        "indcheckxmin":false, "indkey":[1,2], "indclass":["1978","3126"], "indcollation":["0","100"],
        "indoption":[0,1], "indpred":"{CONST :constvalue 8 [ 42 0 0 0 0 0 0 0 ]}", "relnatts":2},
        "owner":"postgres"});
    let original_text = original.to_string();
    let original_record = Record {
        value: &original,
        text: &original_text,
    };
    for (field, replacement) in [
        ("relowner", json!("11")),
        ("relacl", json!([])),
        ("relrowsecurity", json!(false)),
        ("relforcerowsecurity", json!(false)),
        ("indisvalid", json!(false)),
        ("indisready", json!(false)),
        ("indcheckxmin", json!(true)),
        ("indkey", json!([2, 1])),
        ("indclass", json!(["3126", "1978"])),
        ("indcollation", json!(["100", "0"])),
        ("indoption", json!([1, 0])),
        (
            "indpred",
            json!("{CONST :constvalue 8 [ 43 0 0 0 0 0 0 0 ]}"),
        ),
    ] {
        let mut drift = original.clone();
        drift["raw"][field] = replacement;
        drift["raw"]["oid"] = json!("29000");
        let drift_text = drift.to_string();
        let record = Record {
            value: &drift,
            text: &drift_text,
        };
        let mut expected = semantic(original_record);
        let mut actual = semantic(record);
        replace(&mut expected, "raw.oid", &json!("public.declared"));
        replace(&mut actual, "raw.oid", &json!("public.declared"));
        assert_ne!(expected, actual, "identity mapping must retain {field}");
        assert!(catch_unwind(|| exact(original_record, record, "original metadata")).is_err());
    }
    let mut owner = original.clone();
    owner["owner"] = json!("other_owner");
    let owner_text = owner.to_string();
    assert_ne!(
        semantic(original_record),
        semantic(Record {
            value: &owner,
            text: &owner_text
        })
    );
}

#[test]
fn unmapped_numeric_precision_stays_lossless() {
    let first = r#"{"raw":{"constant":1.000000000000000000000000001}}"#;
    let second = r#"{"raw":{"constant":1.000000000000000000000000002}}"#;
    let a: Value = serde_json::from_str(first).unwrap();
    let b: Value = serde_json::from_str(second).unwrap();
    assert_ne!(
        semantic(Record {
            value: &a,
            text: first
        }),
        semantic(Record {
            value: &b,
            text: second
        })
    );
}

#[test]
fn index_order_include_boundaries_and_witness_positions_are_strict() {
    let raw = json!({"indnatts":3,"indnkeyatts":2,"indkey":[1,2,3],"indclass":["1978","3126"],
        "indcollation":["0","100"],"indoption":[0,1]});
    let columns = json!([{"position":1,"attnum":1,"identity":object("a")},
        {"position":2,"attnum":2,"identity":object("b")},{"position":3,"attnum":3,"identity":object("c")}]);
    indexes::ordered_columns(&raw, &columns);
    let mut reordered = raw.clone();
    reordered["indkey"] = json!([2, 1, 3]);
    assert!(catch_unwind(|| indexes::ordered_columns(&reordered, &columns)).is_err());
    let mut boundary = raw;
    boundary["indnkeyatts"] = json!(3);
    assert!(catch_unwind(|| indexes::ordered_columns(&boundary, &columns)).is_err());
}

#[test]
fn array_links_require_both_reciprocal_identities() {
    let row = json!({"oid":"17000","typarray":"17001"});
    let array = json!({"oid":"17001","typelem":"17000"});
    types::reciprocal(&row, &array);
    let mut wrong = array;
    wrong["typelem"] = json!("19000");
    assert!(catch_unwind(|| types::reciprocal(&row, &wrong)).is_err());
    wrong["typelem"] = json!("17000");
    wrong["oid"] = json!("19001");
    assert!(catch_unwind(|| types::reciprocal(&row, &wrong)).is_err());
}

#[test]
fn regproc_witnesses_reject_missing_reordered_or_unresolved_handlers() {
    let fields = [
        "typanalyze",
        "typinput",
        "typmodin",
        "typmodout",
        "typoutput",
        "typreceive",
        "typsend",
        "typsubscript",
    ];
    let mut raw = serde_json::Map::new();
    let helpers: Vec<_> = fields
        .iter()
        .map(|field| {
            raw.insert((*field).to_owned(), json!("-"));
            json!({"field":field,"oid":"0","rendered":"-","identity":null})
        })
        .collect();
    let value = json!({"raw":raw,"handlers":helpers});
    types::handlers(&value);
    let mut reordered = value.clone();
    reordered["handlers"].as_array_mut().unwrap().swap(0, 1);
    assert!(catch_unwind(|| types::handlers(&reordered)).is_err());
    let mut missing = value.clone();
    missing["handlers"].as_array_mut().unwrap().pop();
    assert!(catch_unwind(|| types::handlers(&missing)).is_err());
    let mut unresolved = value;
    unresolved["raw"]["typinput"] = json!("unknown");
    unresolved["handlers"][1] = json!({"field":"typinput","oid":"19000","rendered":"unknown",
        "identity":{"type":"function","schema":"pg_catalog","name":null,"identity":"pg_catalog.unknown()"}});
    assert!(catch_unwind(|| types::handlers(&unresolved)).is_err());
}

#[test]
fn secret_mount_preservation_accepts_acl_spacing_and_rejects_metadata_drift() {
    let original = json!({"namespace":"public","owner":"postgres",
        "identity":{"identity":"public.secret_runtime_mounts"},
        "raw":{"oid":"17000","relkind":"r","relowner":"10","relacl":null,
            "relrowsecurity":false}});
    let original_text = original.to_string();
    let original_record = Record {
        value: &original,
        text: &original_text,
    };
    let mut following = original.clone();
    following["raw"]["relacl"] = json!([
        "postgres=arwdDxtm/postgres",
        "hephaestus_worker=arw/postgres"
    ]);
    let compact = following.to_string();
    let spaced = compact.replace(
        "\"postgres=arwdDxtm/postgres\",\"hephaestus_worker=arw/postgres\"",
        "\"postgres=arwdDxtm/postgres\", \"hephaestus_worker=arw/postgres\"",
    );
    assert_ne!(compact, spaced);
    for text in [&compact, &spaced] {
        let value: Value = serde_json::from_str(text).unwrap();
        assert_eq!(value, following);
        relations::preserve_secret_mounts(
            original_record,
            Record {
                value: &value,
                text,
            },
        );
    }
    let contradictory = compact.replace(
        "hephaestus_worker=arw/postgres",
        "hephaestus_worker=arwd/postgres",
    );
    assert_ne!(contradictory, compact);
    assert!(
        catch_unwind(|| relations::preserve_secret_mounts(
            original_record,
            Record {
                value: &following,
                text: &contradictory
            }
        ))
        .is_err(),
        "typed SIU must not hide DELETE in captured ACL text"
    );
    for (parent, field, value) in [
        (
            "raw",
            "relacl",
            json!([
                "postgres=arwdDxtm/postgres",
                "hephaestus_worker=arwd/postgres"
            ]),
        ),
        ("", "owner", json!("other_owner")),
        ("raw", "relowner", json!("20")),
        ("raw", "oid", json!("17001")),
        ("identity", "identity", json!("public.other_table")),
        ("raw", "relrowsecurity", json!(true)),
    ] {
        let mut drift = following.clone();
        if parent.is_empty() {
            drift[field] = value;
        } else {
            drift[parent][field] = value;
        }
        let text = drift.to_string();
        assert!(
            catch_unwind(|| relations::preserve_secret_mounts(
                original_record,
                Record {
                    value: &drift,
                    text: &text
                }
            ))
            .is_err(),
            "must reject {parent}.{field}"
        );
    }
}
