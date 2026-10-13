//! Complete trigger upgrade inventory; source/native journals remain prerequisites.
#[path = "triggers/links.rs"]
mod links;
#[path = "triggers/ri.rs"]
mod ri;
#[path = "triggers/source.rs"]
mod source;
#[cfg(test)]
#[path = "triggers/tests.rs"]
mod tests;

use super::{Inventory, Record, Semantic, Structure, exact, identity, records, replace, semantic};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

const REMOVED: &str =
    "public.agent_instance_volume_leases.agent_instance_volume_leases_volume_id_instance_id_fkey";
const FIELDS: &[&str] = &[
    "oid",
    "tgrelid",
    "tgparentid",
    "tgname",
    "tgfoid",
    "tgtype",
    "tgenabled",
    "tgisinternal",
    "tgconstrrelid",
    "tgconstrindid",
    "tgconstraint",
    "tgdeferrable",
    "tginitdeferred",
    "tgnargs",
    "tgattr",
    "tgargs",
    "tgqual",
    "tgoldtable",
    "tgnewtable",
];
const SLOTS: [&str; 4] = ["a_delete", "a_update", "c_insert", "c_update"];

fn foreign_keys(constraints: &Inventory<'_>) -> BTreeSet<String> {
    constraints
        .iter()
        .filter(|(_, entry)| entry.value["raw"]["contype"] == "f")
        .map(|(key, _)| key.clone())
        .collect()
}

fn ri_key(constraint: &str, slot: &str) -> String {
    format!("RI({constraint},{slot})")
}

fn inventory<'a>(structure: &Structure<'a>, constraints: &Inventory<'_>) -> Inventory<'a> {
    let mut result = BTreeMap::new();
    let mut generated = BTreeSet::new();
    for record in records(structure.snapshot, "triggers") {
        let value = record.value;
        identity::keys(
            value,
            &[
                "raw",
                "identity",
                "relation",
                "function",
                "constraint",
                "constraint_relation",
                "definition",
            ],
        );
        let raw = &value["raw"];
        identity::keys(raw, FIELDS);
        assert_eq!(
            identity::oid(&raw["tgparentid"]),
            "0",
            "unpartitioned source trigger"
        );
        let relation = &structure.classes[identity::oid(&raw["tgrelid"])];
        assert!(relation.starts_with("public."));
        assert_eq!(structure.relations[relation].value["raw"]["relkind"], "r");
        identity::class_link(structure, &raw["tgrelid"], &value["relation"]);
        identity::class_link(
            structure,
            &raw["tgconstrrelid"],
            &value["constraint_relation"],
        );
        let constraint = links::constraint(constraints, &raw["tgconstraint"], &value["constraint"]);
        let name = identity::text(&raw["tgname"]);
        links::trigger_identity(&value["identity"], name, &value["relation"]);
        let key = if raw["tgisinternal"].as_bool().unwrap() {
            let (key, constraint) = constraint.expect("internal FK constraint");
            assert_eq!(
                constraint.value["raw"]["contype"], "f",
                "only source RI generators"
            );
            let (slot, function) = ri::role(raw, &constraint.value["raw"]);
            ri::name(raw, slot);
            ri::function(&value["function"], function);
            let key = ri_key(&key, slot);
            assert!(generated.insert(key.clone()));
            key
        } else {
            links::public_function(structure, &raw["tgfoid"], &value["function"]);
            if let Some((_, constraint)) = constraint {
                assert_eq!(constraint.value["raw"]["contype"], "t");
                assert_eq!(constraint.value["raw"]["conrelid"], raw["tgrelid"]);
                assert_eq!(constraint.value["raw"]["conname"], raw["tgname"]);
            }
            format!("{relation}.{}", source::identifier(name))
        };
        assert!(
            result.insert(key, record).is_none(),
            "duplicate trigger identity"
        );
    }
    let expected: BTreeSet<_> = foreign_keys(constraints)
        .iter()
        .flat_map(|key| SLOTS.map(|slot| ri_key(key, slot)))
        .collect();
    assert_eq!(
        generated, expected,
        "complete four-trigger FK implementation"
    );
    assert_eq!(identity::ids(&result, "oid").len(), result.len());
    result
}

fn normalized(
    structure: &Structure<'_>,
    key: &str,
    record: Record<'_>,
    constraints: &Inventory<'_>,
) -> Semantic {
    let value = record.value;
    let raw = &value["raw"];
    let mut result = semantic(record);
    replace(&mut result, "raw.oid", &json!(key));
    for field in ["tgrelid", "tgconstrrelid", "tgconstrindid"] {
        replace(
            &mut result,
            &format!("raw.{field}"),
            &links::class(structure, &raw[field]),
        );
    }
    let constraint = links::constraint(constraints, &raw["tgconstraint"], &value["constraint"]);
    replace(
        &mut result,
        "raw.tgconstraint",
        &json!(constraint.as_ref().map_or("0", |(key, _)| key.as_str())),
    );
    if raw["tgisinternal"] == true {
        let (_, constraint) = constraint.unwrap();
        let (slot, _) = ri::role(raw, &constraint.value["raw"]);
        let name = ri::name(raw, slot);
        replace(&mut result, "raw.tgname", &json!(key));
        let mut helper = value["identity"].clone();
        helper["identity"] = json!(key);
        replace(&mut result, "identity", &helper);
        replace(
            &mut result,
            "definition",
            &ri::definition(record, &name, key),
        );
    } else {
        let (binding, _) = links::public_function(structure, &raw["tgfoid"], &value["function"]);
        replace(&mut result, "raw.tgfoid", &binding);
    }
    result
}

pub(super) fn compare(old: &Structure<'_>, upgraded: &Structure<'_>, fresh: &Structure<'_>) {
    // Reuse actual independent source-group admission, not a copied FK allowlist.
    super::super::catalog_constraints::assert_constraints(
        old.snapshot,
        upgraded.snapshot,
        fresh.snapshot,
    );
    let declarations = source::additions();
    let old_constraints = links::constraints(old);
    let current_constraints = links::constraints(upgraded);
    let fresh_constraints = links::constraints(fresh);
    assert!(foreign_keys(&old_constraints).contains(REMOVED));
    assert!(!fresh_constraints.contains_key(REMOVED));
    let original = inventory(old, &old_constraints);
    let current = inventory(upgraded, &current_constraints);
    let canonical = inventory(fresh, &fresh_constraints);
    let removed: BTreeSet<_> = SLOTS
        .map(|slot| ri_key(REMOVED, slot))
        .into_iter()
        .collect();
    assert!(removed.iter().all(|key| original.contains_key(key)));
    let mut additions: BTreeSet<_> = declarations.keys().cloned().collect();
    assert!(additions.iter().all(|key| !original.contains_key(key)));
    for key in foreign_keys(&fresh_constraints).difference(&foreign_keys(&old_constraints)) {
        for slot in SLOTS {
            assert!(additions.insert(ri_key(key, slot)));
        }
    }
    for (key, declaration) in declarations {
        let record = canonical[&key];
        let (_, function) = links::public_function(
            fresh,
            &record.value["raw"]["tgfoid"],
            &record.value["function"],
        );
        assert_eq!(function, declaration.function);
        assert_eq!(record.value["raw"]["tgisinternal"], false);
        assert_eq!(
            identity::oid(&record.value["raw"]["tgconstraint"]) != "0",
            declaration.constraint
        );
    }
    let expected: BTreeSet<_> = original
        .keys()
        .filter(|key| !removed.contains(*key))
        .cloned()
        .chain(additions.iter().cloned())
        .collect();
    assert_inventory(&current, &expected);
    assert_inventory(&canonical, &expected);
    let old_ids = identity::ids(&original, "oid");
    for (key, record) in &original {
        if !removed.contains(key) {
            exact(*record, current[key], "original trigger changed");
        }
    }
    for (key, record) in &current {
        if additions.contains(key) {
            assert!(!old_ids.contains_key(identity::oid(&record.value["raw"]["oid"])));
        }
        assert_semantics(
            &normalized(upgraded, key, *record, &current_constraints),
            &normalized(fresh, key, canonical[key], &fresh_constraints),
        );
    }
}

fn assert_inventory(inventory: &Inventory<'_>, expected: &BTreeSet<String>) {
    assert_eq!(
        &inventory.keys().cloned().collect::<BTreeSet<_>>(),
        expected,
        "complete source trigger inventory"
    );
}

fn assert_semantics(actual: &Semantic, expected: &Semantic) {
    assert_eq!(actual, expected, "full canonical trigger semantics");
}

// Reuse the qualified inventory for typed dependency endpoints; no separate RI policy.
pub(super) fn dependency_inventory<'a>(structure: &Structure<'a>) -> Inventory<'a> {
    let constraints = links::constraints(structure);
    inventory(structure, &constraints)
}
