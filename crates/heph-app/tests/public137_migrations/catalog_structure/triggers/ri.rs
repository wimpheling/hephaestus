//! Finite `PostgreSQL` 17 FK trigger semantics; no generic generated-name rewriting.
use super::super::{Record, identity};
use serde_json::{Value, json};

pub(super) fn role(raw: &Value, constraint: &Value) -> (&'static str, &'static str) {
    let (slot, function, expected_id) = match raw["tgtype"].as_u64().unwrap() {
        5 => ("c_insert", "RI_FKey_check_ins", "1644"),
        17 if raw["tgrelid"] == constraint["conrelid"] && raw["tgfoid"] == "1645" => {
            ("c_update", "RI_FKey_check_upd", "1645")
        }
        9 => action(identity::text(&constraint["confdeltype"]), false),
        17 => action(identity::text(&constraint["confupdtype"]), true),
        _ => panic!("unsupported RI trigger event"),
    };
    assert_eq!(raw["tgfoid"], expected_id);
    let check = slot.starts_with('c');
    assert_eq!(
        raw["tgrelid"],
        constraint[if check { "conrelid" } else { "confrelid" }]
    );
    assert_eq!(
        raw["tgconstrrelid"],
        constraint[if check { "confrelid" } else { "conrelid" }]
    );
    assert_eq!(raw["tgconstrindid"], constraint["conindid"]);
    let deferred = check || function.contains("noaction");
    assert_eq!(
        raw["tgdeferrable"],
        if deferred {
            constraint["condeferrable"].clone()
        } else {
            json!(false)
        }
    );
    assert_eq!(
        raw["tginitdeferred"],
        if deferred {
            constraint["condeferred"].clone()
        } else {
            json!(false)
        }
    );
    assert_eq!(raw["tgnargs"], 0);
    assert!(raw["tgqual"].is_null());
    (slot, function)
}

fn action(action: &str, update: bool) -> (&'static str, &'static str, &'static str) {
    match (action, update) {
        ("c", false) => ("a_delete", "RI_FKey_cascade_del", "1646"),
        ("c", true) => ("a_update", "RI_FKey_cascade_upd", "1647"),
        ("r", false) => ("a_delete", "RI_FKey_restrict_del", "1648"),
        ("r", true) => ("a_update", "RI_FKey_restrict_upd", "1649"),
        ("n", false) => ("a_delete", "RI_FKey_setnull_del", "1650"),
        ("n", true) => ("a_update", "RI_FKey_setnull_upd", "1651"),
        ("d", false) => ("a_delete", "RI_FKey_setdefault_del", "1652"),
        ("d", true) => ("a_update", "RI_FKey_setdefault_upd", "1653"),
        ("a", false) => ("a_delete", "RI_FKey_noaction_del", "1654"),
        ("a", true) => ("a_update", "RI_FKey_noaction_upd", "1655"),
        _ => panic!("unsupported FK action"),
    }
}

pub(super) fn name(raw: &Value, slot: &str) -> String {
    let oid = identity::oid(&raw["oid"]);
    assert_ne!(oid, "0");
    let expected = format!(
        "RI_ConstraintTrigger_{}_{}",
        if slot.starts_with('c') { 'c' } else { 'a' },
        oid
    );
    assert_eq!(raw["tgname"], expected, "RI name uses its own trigger OID");
    expected
}

pub(super) fn function(helper: &Value, name: &str) {
    assert_eq!(
        helper,
        &json!({"type": "function", "schema": "pg_catalog", "name": null,
        "identity": format!("pg_catalog.\"{name}\"()") })
    );
}

pub(super) fn definition(record: Record<'_>, name: &str, key: &str) -> Value {
    let prefix = format!("CREATE CONSTRAINT TRIGGER \"{name}\" ");
    let tail = identity::text(&record.value["definition"])
        .strip_prefix(&prefix)
        .expect("exact RI deparser prefix");
    json!(format!("CREATE CONSTRAINT TRIGGER {key} {tail}"))
}
