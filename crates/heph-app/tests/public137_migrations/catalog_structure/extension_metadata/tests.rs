use super::{
    Inventory, Record, comments, compare_domain, embedded_extension, extensions, members, profile,
    semantic, signed,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn native() -> Value {
    json!({"raw":{"oid":"20000","extname":"pgcrypto","extowner":"10","extnamespace":"2200",
        "extrelocatable":true,"extversion":"1.3","extconfig":null,"extcondition":null},
        "owner":"postgres","namespace":{"oid":"2200","nspname":"public","nspowner":"6171",
            "nspacl":["pg_database_owner=UC/pg_database_owner","=U/pg_database_owner"]},
        "config_relations":null})
}

fn member() -> Value {
    json!({"raw":{"classid":"1255","objid":"20001","objsubid":0,"refclassid":"3079",
        "refobjid":"20000","refobjsubid":0,"deptype":"e"},
        "extension":native()["raw"],
        "identity":{"type":"function","schema":"public","name":null,"identity":"public.digest(text, text)"},
        "address":{"type":"function","object_names":["public","digest"],"object_args":["text","text"]}})
}

fn comment() -> Value {
    json!({"raw":{"objoid":"20000","classoid":"3079","objsubid":0,"description":"cryptographic functions"},
        "identity":{"type":"extension","schema":null,"name":"pgcrypto","identity":"pgcrypto"}})
}

#[test]
fn normal_sources_and_existing_native_seals_are_bound() {
    profile::qualify();
}

#[test]
fn native_owner_namespace_version_and_configuration_substitutions_are_denied() {
    let original = native();
    extensions::native(&original);
    for (section, field, replacement) in [
        ("raw", "extname", json!("other")),
        ("raw", "extversion", json!("1.4")),
        ("raw", "extrelocatable", json!(false)),
        ("raw", "extconfig", json!(["20099"])),
        ("raw", "extcondition", json!(["true"])),
        ("namespace", "nspname", json!("other")),
    ] {
        let mut changed = original.clone();
        changed[section][field] = replacement;
        assert!(std::panic::catch_unwind(|| extensions::native(&changed)).is_err());
    }
    for field in ["owner", "config_relations"] {
        let mut changed = original.clone();
        changed[field] = json!("other");
        assert!(std::panic::catch_unwind(|| extensions::native(&changed)).is_err());
    }
}

#[test]
fn native_extension_preserves_namespace_acl_owner_and_all_raw_fields() {
    let original = native();
    let text = original.to_string();
    let expected = extensions::normalized(Record {
        value: &original,
        text: &text,
    });
    for (section, field, replacement) in [
        ("raw", "extowner", json!("20")),
        ("raw", "extnamespace", json!("9999")),
        ("namespace", "nspowner", json!("20")),
        ("namespace", "nspacl", json!(null)),
        ("raw", "extcondition", json!(["quoted full condition"])),
    ] {
        let mut changed = original.clone();
        changed[section][field] = replacement;
        let changed_text = changed.to_string();
        assert_ne!(
            expected,
            extensions::normalized(Record {
                value: &changed,
                text: &changed_text
            })
        );
    }
}

#[test]
fn every_member_catalog_and_subobject_slot_is_checked() {
    let original = member();
    members::shape(&original["raw"]);
    for (field, replacement) in [
        ("classid", json!("1259")),
        ("objid", json!("0")),
        ("objsubid", json!(-1)),
        ("refclassid", json!("1255")),
        ("refobjid", json!("0")),
        ("refobjsubid", json!(1)),
        ("deptype", json!("n")),
    ] {
        let mut changed = original["raw"].clone();
        changed[field] = replacement;
        assert!(std::panic::catch_unwind(|| members::shape(&changed)).is_err());
    }
    let mut changed = original["raw"].clone();
    changed["unknown"] = json!(0);
    assert!(std::panic::catch_unwind(|| members::shape(&changed)).is_err());
}

#[test]
fn member_translation_preserves_address_and_embedded_extension_metadata() {
    let original = member();
    let text = original.to_string();
    let expected = members::normalized(Record {
        value: &original,
        text: &text,
    });
    for (section, field, replacement) in [
        ("extension", "extowner", json!("20")),
        ("extension", "extcondition", json!(["x"])),
        ("extension", "extconfig", json!(["20099"])),
        ("address", "object_args", json!(["text", "bytea"])),
        ("raw", "refobjsubid", json!(1)),
    ] {
        let mut changed = original.clone();
        changed[section][field] = replacement;
        let changed_text = changed.to_string();
        assert_ne!(
            expected,
            members::normalized(Record {
                value: &changed,
                text: &changed_text
            })
        );
    }
    let mut renumbered = original;
    renumbered["raw"]["objid"] = json!("30001");
    renumbered["raw"]["refobjid"] = json!("30000");
    renumbered["extension"]["oid"] = json!("30000");
    let renumbered_text = renumbered.to_string();
    assert_eq!(
        expected,
        members::normalized(Record {
            value: &renumbered,
            text: &renumbered_text
        })
    );
}

#[test]
fn unknown_embedded_extension_fields_and_lossy_text_are_denied() {
    let mut changed = member();
    changed["extension"]["unknown"] = json!(0);
    let changed_text = changed.to_string();
    assert!(
        std::panic::catch_unwind(|| {
            let record = Record {
                value: &changed,
                text: &changed_text,
            };
            embedded_extension(&mut semantic(record), record);
        })
        .is_err()
    );
    let mut original = member();
    original["extension"]["extcondition"] = json!([1.0]);
    let text = original.to_string();
    let unusual = text.replace("[1.0]", "[1.00]");
    assert_eq!(serde_json::from_str::<Value>(&unusual).unwrap(), original);
    assert_ne!(text, unusual);
    assert_ne!(
        members::normalized(Record {
            value: &original,
            text: &text
        }),
        members::normalized(Record {
            value: &original,
            text: &unusual
        })
    );
}

#[test]
fn native_comment_text_identity_class_and_subobject_substitutions_are_denied() {
    let original = comment();
    comments::native_comment(&original);
    for (section, field, replacement) in [
        ("raw", "description", json!("cryptographic functions\n")),
        ("raw", "classoid", json!("1255")),
        ("raw", "objsubid", json!(-1)),
        ("identity", "name", json!("other")),
        ("identity", "identity", json!("other")),
        ("identity", "schema", json!("public")),
    ] {
        let mut changed = original.clone();
        changed[section][field] = replacement;
        assert!(std::panic::catch_unwind(|| comments::native_comment(&changed)).is_err());
    }
}

#[test]
fn signed_subobjects_are_not_unsigned_or_string_coerced() {
    assert_eq!(signed(&json!(-7)), -7);
    assert_eq!(signed(&json!(0)), 0);
    for invalid in [json!("-7"), json!(1.5), json!(2_147_483_648_i64)] {
        assert!(std::panic::catch_unwind(|| signed(&invalid)).is_err());
    }
}

#[test]
fn inherited_records_and_finite_inventory_cannot_be_replaced_by_fresh_target_rows() {
    let original = comment();
    let text = original.to_string();
    let mut old = Inventory::new();
    old.insert(
        "native".to_owned(),
        Record {
            value: &original,
            text: &text,
        },
    );
    let expected = BTreeSet::from(["native".to_owned()]);
    compare_domain(&old, &old, &old, &expected, semantic, semantic);
    let mut changed = original.clone();
    changed["raw"]["objoid"] = json!("30000");
    let changed_text = changed.to_string();
    let mut current = Inventory::new();
    current.insert(
        "native".to_owned(),
        Record {
            value: &changed,
            text: &changed_text,
        },
    );
    assert!(
        std::panic::catch_unwind(|| compare_domain(
            &old, &current, &current, &expected, semantic, semantic
        ))
        .is_err()
    );
    let empty = Inventory::new();
    assert!(
        std::panic::catch_unwind(|| compare_domain(
            &old, &empty, &old, &expected, semantic, semantic
        ))
        .is_err()
    );
    current.insert(
        "arbitrary".to_owned(),
        Record {
            value: &original,
            text: &text,
        },
    );
    assert!(
        std::panic::catch_unwind(|| compare_domain(
            &old, &current, &current, &expected, semantic, semantic
        ))
        .is_err()
    );
}

#[test]
fn inherited_plpgsql_language_and_handlers_require_exact_captured_lineage() {
    let language = json!({"raw":{"oid":"13563","lanplcallfoid":"13560","laninline":"13561","lanvalidator":"13562"},
        "call_handler":{"type":"function","schema":"pg_catalog","name":null,"identity":"pg_catalog.plpgsql_call_handler()"}});
    let mut entry = member();
    entry["extension"]["extname"] = json!("plpgsql");
    entry["raw"]["classid"] = json!("2612");
    entry["raw"]["objid"] = json!("13563");
    entry["identity"] =
        json!({"type":"language","schema":null,"name":"plpgsql","identity":"plpgsql"});
    entry["address"] = json!({"type":"language","object_names":["plpgsql"],"object_args":[]});
    members::shape(&entry["raw"]);
    members::plpgsql_member(&entry, &language);
    for (section, field, replacement) in [
        ("raw", "classid", json!("1255")),
        ("raw", "objid", json!("13564")),
        ("identity", "schema", json!("pg_catalog")),
        ("identity", "type", json!("function")),
        ("identity", "name", json!("other")),
        ("address", "object_args", json!(["oid"])),
    ] {
        let mut changed = entry.clone();
        changed[section][field] = replacement;
        assert!(std::panic::catch_unwind(|| members::plpgsql_member(&changed, &language)).is_err());
    }
    entry["raw"]["classid"] = json!("1255");
    entry["raw"]["objid"] = json!("13560");
    entry["identity"] = language["call_handler"].clone();
    entry["address"] = json!({"type":"function","object_names":["pg_catalog","plpgsql_call_handler"],"object_args":[]});
    members::plpgsql_member(&entry, &language);
    for (section, field, replacement) in [
        ("raw", "classid", json!("2612")),
        ("raw", "objid", json!("13564")),
        ("identity", "schema", json!("public")),
        (
            "address",
            "object_names",
            json!(["pg_catalog", "unrelated_handler"]),
        ),
        ("address", "object_args", json!(["pg_catalog.oid"])),
    ] {
        let mut changed = entry.clone();
        changed[section][field] = replacement;
        assert!(std::panic::catch_unwind(|| members::plpgsql_member(&changed, &language)).is_err());
    }
}
