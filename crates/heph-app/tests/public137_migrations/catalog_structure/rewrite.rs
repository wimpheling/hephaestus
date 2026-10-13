//! Source 108 adds a STORED generated column to this existing parent. `PostgreSQL`
//! 17 rewrites its heap, swaps the TOAST link and renames the new TOAST children
//! for the surviving parent OID. No other original physical identity may change.
use super::{Semantic, Structure, identity, indexes, relations, replace};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[cfg(test)]
#[path = "rewrite/tests.rs"]
mod tests;

pub(super) const PARENT: &str = "public.agent_instance_volume_leases";
const COLUMN: &str = "named_binding_revision_id";

pub(super) fn children() -> BTreeSet<String> {
    BTreeSet::from([format!("toast({PARENT})"), format!("toast_index({PARENT})")])
}

// Earlier source-qualified column/default assertions protect the generated
// expression. This witness ties the physical exception to that exact addition.
fn generated_column(structure: &Structure<'_>, present: bool) {
    let parent = structure.relations[PARENT].value;
    let matches: Vec<_> = identity::rows(structure.snapshot, "columns")
        .iter()
        .filter(|column| {
            column["raw"]["attrelid"] == parent["raw"]["oid"] && column["raw"]["attname"] == COLUMN
        })
        .collect();
    assert_eq!(matches.len(), usize::from(present));
    if present {
        let column = matches[0];
        assert_eq!(column["relation"], parent["identity"]);
        assert_eq!(column["raw"]["attnum"], 26);
        assert_eq!(column["raw"]["attgenerated"], "s");
        assert_eq!(column["raw"]["atthasdef"], true);
        assert_eq!(column["raw"]["attisdropped"], false);
        assert_eq!(column["raw"]["atttypid"], "2950");
        assert_eq!(
            column["type"],
            json!({"type":"type", "schema":"pg_catalog", "name":"uuid", "identity":"pg_catalog.uuid"})
        );
    }
}

fn graph(parent: &Value, toast: &Value, index_class: &Value, index: &Value) {
    let parent_identity = json!({"type":"table", "schema":"public", "name":"agent_instance_volume_leases", "identity":PARENT});
    assert_eq!(parent["identity"], parent_identity);
    assert_eq!(parent["namespace"], "public");
    assert_eq!(parent["raw"]["relkind"], "r");
    assert_eq!(parent["raw"]["relname"], "agent_instance_volume_leases");
    assert_eq!(parent["raw"]["reltoastrelid"], toast["raw"]["oid"]);
    assert_ne!(identity::oid(&toast["raw"]["oid"]), "0");
    assert_ne!(toast["raw"]["oid"], index_class["raw"]["oid"]);
    for (child, is_index) in [(toast, false), (index_class, true)] {
        assert_eq!(child["namespace"], "pg_toast");
        assert_eq!(child["raw"]["relkind"], if is_index { "i" } else { "t" });
        assert_eq!(identity::oid(&child["raw"]["reltype"]), "0");
        assert_eq!(identity::oid(&child["raw"]["reltoastrelid"]), "0");
        assert_eq!(
            child[if is_index {
                "toast_index_parent"
            } else {
                "toast_parent"
            }],
            parent_identity
        );
        let name = identity::generated_name(identity::oid(&parent["raw"]["oid"]), is_index);
        assert_eq!(child["raw"]["relname"], name);
        identity::toast_identity(
            &child["identity"],
            &name,
            &format!(
                "{}({PARENT})",
                if is_index { "toast_index" } else { "toast" }
            ),
        );
    }
    assert_eq!(index["raw"]["indexrelid"], index_class["raw"]["oid"]);
    assert_eq!(index["raw"]["indrelid"], toast["raw"]["oid"]);
    assert_eq!(index["identity"], index_class["identity"]);
    assert_eq!(index["table"], toast["identity"]);
}

pub(super) fn qualify(
    old: &Structure<'_>,
    upgraded: &Structure<'_>,
    fresh: &Structure<'_>,
) -> BTreeSet<String> {
    generated_column(old, false);
    generated_column(upgraded, true);
    generated_column(fresh, true);
    assert_eq!(
        old.relations[PARENT].value["raw"]["oid"],
        upgraded.relations[PARENT].value["raw"]["oid"]
    );
    assert_eq!(
        old.relations[PARENT].value["raw"]["reltype"],
        upgraded.relations[PARENT].value["raw"]["reltype"]
    );
    let toast = format!("toast({PARENT})");
    let index = format!("toast_index({PARENT})");
    let mut index_semantics = Vec::new();
    for structure in [old, upgraded, fresh] {
        let indexes = indexes::inventory(structure);
        graph(
            structure.relations[PARENT].value,
            structure.relations[&toast].value,
            structure.relations[&index].value,
            indexes[&index].value,
        );
        // Full index normalization verifies the finite definition, ordered
        // attributes, builtin witnesses and every validity/visibility flag.
        index_semantics.push(indexes::normalized(structure, &index, indexes[&index]));
    }
    assert_eq!(
        index_semantics[0], index_semantics[1],
        "complete rewritten index preservation"
    );
    assert_eq!(
        index_semantics[1], index_semantics[2],
        "canonical rewritten index"
    );
    let replaced = children();
    for key in &replaced {
        let new_oid = identity::oid(&upgraded.relations[key].value["raw"]["oid"]);
        assert!(
            !old.classes.contains_key(new_oid),
            "replacement must have a new class identity"
        );
        assert_eq!(
            relations::normalized(old, key),
            relations::normalized(upgraded, key),
            "complete rewritten child preservation {key}"
        );
        assert_eq!(
            relations::normalized(upgraded, key),
            relations::normalized(fresh, key),
            "canonical rewritten child {key}"
        );
    }
    replaced
}

// Map only this qualified parent field through its captured class inventory.
// Parent OID, row type, ownership, ACL and all other raw fields stay untouched.
pub(super) fn parent_link(structure: &Structure<'_>, result: &mut Semantic) {
    let parent = structure.relations[PARENT].value;
    let key = &structure.classes[identity::oid(&parent["raw"]["reltoastrelid"])];
    assert_eq!(key, &format!("toast({PARENT})"));
    replace(result, "raw.reltoastrelid", &json!(key));
}
