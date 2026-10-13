//! Full inherited policies and independently source-admitted fresh semantics.
#[path = "policies/ast.rs"]
mod ast;
#[path = "policies/shapes.rs"]
mod shapes;
#[path = "policies/source.rs"]
mod source;
#[cfg(test)]
#[path = "policies/tests.rs"]
mod tests;

use super::{Inventory, Record, Semantic, Structure, exact, identity, records, replace, semantic};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn role_names(structure: &Structure<'_>, value: &Value) -> BTreeSet<String> {
    let roles = value["roles"].as_array().unwrap();
    let mut raw: Vec<_> = value["raw"]["polroles"]
        .as_array()
        .unwrap()
        .iter()
        .map(identity::oid)
        .collect();
    raw.sort_unstable_by_key(|id| id.parse::<u32>().unwrap());
    assert!(!raw.is_empty());
    assert_eq!(raw.len(), roles.len());
    let mut names = BTreeSet::new();
    for (id, role) in raw.into_iter().zip(roles) {
        identity::keys(role, &["oid", "name"]);
        assert_eq!(identity::oid(&role["oid"]), id, "ordered role OID witness");
        if id == "0" {
            assert_public_role(&role["oid"], &role["name"]);
        } else {
            identity::owner(structure.snapshot, &role["oid"], &role["name"]);
        }
        assert!(
            names.insert(identity::text(&role["name"]).into()),
            "duplicate policy role"
        );
    }
    names
}

fn inventory<'a>(structure: &Structure<'a>) -> Inventory<'a> {
    let mut result = BTreeMap::new();
    for record in records(structure.snapshot, "policies") {
        let value = record.value;
        identity::keys(
            value,
            &["raw", "identity", "table", "roles", "using", "check"],
        );
        let raw = &value["raw"];
        identity::keys(
            raw,
            &[
                "oid",
                "polname",
                "polrelid",
                "polcmd",
                "polpermissive",
                "polroles",
                "polqual",
                "polwithcheck",
            ],
        );
        let relation = &structure.classes[identity::oid(&raw["polrelid"])];
        assert!(relation.starts_with("public."));
        assert_eq!(structure.relations[relation].value["raw"]["relkind"], "r");
        identity::class_link(structure, &raw["polrelid"], &value["table"]);
        let name = source::identifier(identity::text(&raw["polname"]));
        identity::object(&value["identity"]);
        assert_eq!(
            value["identity"],
            json!({"type": "policy", "schema": null, "name": null,
            "identity": format!("{name} on {}", identity::text(&value["table"]["identity"]))})
        );
        role_names(structure, value);
        for (expression, deparser) in [("polqual", "using"), ("polwithcheck", "check")] {
            assert_eq!(raw[expression].is_null(), value[deparser].is_null());
            assert!(raw[expression].is_null() || raw[expression].is_string());
            assert!(value[deparser].is_null() || value[deparser].is_string());
        }
        assert!(
            result
                .insert(format!("{relation}.{name}"), record)
                .is_none(),
            "duplicate policy identity"
        );
    }
    assert_eq!(identity::ids(&result, "oid").len(), result.len());
    result
}

fn functions(structure: &Structure<'_>) -> BTreeMap<String, Value> {
    let mut result = BTreeMap::new();
    for routine in identity::rows(structure.snapshot, "routines") {
        identity::object(&routine["identity"]);
        assert_eq!(routine["identity"]["schema"], "public");
        assert!(
            result
                .insert(
                    identity::oid(&routine["raw"]["oid"]).into(),
                    routine["identity"].clone()
                )
                .is_none()
        );
    }
    result
}

fn normalized(
    structure: &Structure<'_>,
    key: &str,
    record: Record<'_>,
    functions: &BTreeMap<String, Value>,
) -> Semantic {
    let mut result = semantic(record);
    replace(&mut result, "raw.oid", &json!(key));
    replace(
        &mut result,
        "raw.polrelid",
        &identity::class_link(
            structure,
            &record.value["raw"]["polrelid"],
            &record.value["table"],
        ),
    );
    let classes: BTreeMap<_, _> = structure
        .classes
        .iter()
        .map(|(id, key)| (id.clone(), json!(key)))
        .collect();
    for field in ["polqual", "polwithcheck"] {
        let raw = &record.value["raw"][field];
        if !raw.is_null() {
            replace(
                &mut result,
                &format!("raw.{field}"),
                &ast::normalized(identity::text(raw), functions, &classes),
            );
        }
    }
    result
}

pub(super) fn compare(old: &Structure<'_>, upgraded: &Structure<'_>, fresh: &Structure<'_>) {
    // Reuse the full qualified routine oracle before supplying AST identities.
    super::super::catalog_routines::assert_routines(
        old.snapshot,
        upgraded.snapshot,
        fresh.snapshot,
    );
    let additions = source::additions();
    let original = inventory(old);
    let current = inventory(upgraded);
    let canonical = inventory(fresh);
    assert!(additions.keys().all(|key| !original.contains_key(key)));
    let expected: BTreeSet<_> = original.keys().chain(additions.keys()).cloned().collect();
    assert_inventory(&current, &expected);
    assert_inventory(&canonical, &expected);
    for (key, declaration) in additions {
        let value = canonical[&key].value;
        assert_eq!(value["raw"]["polcmd"], declaration.command);
        assert_eq!(value["raw"]["polpermissive"], declaration.permissive);
        assert_source_roles(
            fresh.snapshot.context().json(),
            &role_names(fresh, value),
            &declaration.roles,
        );
    }
    for (key, record) in &original {
        exact(*record, current[key], "original policy changed");
    }
    let old_ids = identity::ids(&original, "oid");
    let current_functions = functions(upgraded);
    let fresh_functions = functions(fresh);
    for (key, record) in &current {
        if !original.contains_key(key) {
            assert!(!old_ids.contains_key(identity::oid(&record.value["raw"]["oid"])));
        }
        assert_semantics(
            &normalized(upgraded, key, *record, &current_functions),
            &normalized(fresh, key, canonical[key], &fresh_functions),
        );
    }
}

fn assert_inventory(actual: &Inventory<'_>, expected: &BTreeSet<String>) {
    assert_eq!(
        &actual.keys().cloned().collect::<BTreeSet<_>>(),
        expected,
        "complete source policy inventory"
    );
}

fn assert_semantics(actual: &Semantic, expected: &Semantic) {
    assert_eq!(actual, expected, "complete canonical policy semantics");
}

fn assert_public_role(oid: &Value, name: &Value) {
    assert_eq!(identity::oid(oid), "0");
    assert_eq!(*name, "PUBLIC");
}

fn assert_source_roles(context: &Value, actual: &BTreeSet<String>, declared: &BTreeSet<String>) {
    // fixture::run qualifies the same migration connection as postgres/none;
    // the sealed fresh Snapshot independently qualifies that capture owner.
    for (field, expected) in [
        ("session_user", "postgres"),
        ("current_user", "postgres"),
        ("role", "none"),
    ] {
        assert_eq!(
            context[field], expected,
            "qualified migration/capture owner"
        );
    }
    let expected = declared
        .iter()
        .map(|role| {
            if role == "current_user" {
                context["current_user"].as_str().unwrap().to_owned()
            } else {
                role.clone()
            }
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, &expected, "source policy role admission");
}
