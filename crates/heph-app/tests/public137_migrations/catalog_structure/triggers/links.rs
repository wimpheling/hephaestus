//! Typed joins against captures already qualified by the existing oracles.
use super::super::{Inventory, Structure, identity, records};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) fn constraints<'a>(structure: &Structure<'a>) -> Inventory<'a> {
    let mut result = BTreeMap::new();
    for record in records(structure.snapshot, "constraints") {
        let raw = &record.value["raw"];
        let relation = &structure.classes[identity::oid(&raw["conrelid"])];
        assert!(relation.starts_with("public."));
        assert_eq!(
            record.value["relation"],
            structure.relations[relation].value["identity"]
        );
        let key = format!("{relation}.{}", identity::text(&raw["conname"]));
        assert!(
            result.insert(key, record).is_none(),
            "duplicate qualified constraint"
        );
    }
    assert_eq!(identity::ids(&result, "oid").len(), result.len());
    result
}

pub(super) fn constraint<'a>(
    inventory: &Inventory<'a>,
    raw: &Value,
    helper: &Value,
) -> Option<(String, super::super::Record<'a>)> {
    if identity::oid(raw) == "0" {
        assert!(helper.is_null());
        return None;
    }
    let ids = identity::ids(inventory, "oid");
    let key = ids[identity::oid(raw)].clone();
    let record = inventory[&key];
    assert_eq!(record.value["identity"], *helper);
    Some((key, record))
}

pub(super) fn class(structure: &Structure<'_>, raw: &Value) -> Value {
    let id = identity::oid(raw);
    if id == "0" {
        json!("0")
    } else {
        json!(&structure.classes[id])
    }
}

pub(super) fn public_function(
    structure: &Structure<'_>,
    raw: &Value,
    helper: &Value,
) -> (Value, String) {
    let matches: Vec<_> = identity::rows(structure.snapshot, "routines")
        .iter()
        .filter(|entry| entry["raw"]["oid"] == *raw)
        .collect();
    assert_eq!(matches.len(), 1, "captured public trigger routine");
    let routine = matches[0];
    assert_eq!(routine["identity"], *helper);
    assert_eq!(helper["schema"], "public");
    assert_eq!(routine["raw"]["prokind"], "f");
    assert_eq!(routine["raw"]["prorettype"], "2279");
    assert_eq!(routine["raw"]["pronargs"], 0);
    assert_eq!(routine["raw"]["proargtypes"], json!([]));
    (
        helper.clone(),
        identity::text(&routine["raw"]["proname"]).to_owned(),
    )
}

pub(super) fn trigger_identity(helper: &Value, name: &str, relation: &Value) {
    identity::object(helper);
    assert_eq!(helper["type"], "trigger");
    assert!(helper["schema"].is_null() && helper["name"].is_null());
    let quoted = if name.starts_with("RI_ConstraintTrigger_") {
        format!("\"{name}\"")
    } else {
        super::source::identifier(name).to_owned()
    };
    assert_eq!(
        helper["identity"],
        format!("{quoted} on {}", identity::text(&relation["identity"]))
    );
}
