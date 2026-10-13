use super::{PARENT, graph};
use serde_json::{Value, json};
use std::panic::catch_unwind;

fn fixture(parent_oid: &str, toast_oid: &str, index_oid: &str) -> [Value; 4] {
    let parent_identity = json!({"type":"table", "schema":"public", "name":"agent_instance_volume_leases", "identity":PARENT});
    let name = format!("pg_toast_{parent_oid}");
    let index_name = format!("{name}_index");
    let toast_identity = json!({"type":"toast table", "schema":"pg_toast", "name":name, "identity":format!("pg_toast.{name}")});
    let index_identity = json!({"type":"index", "schema":"pg_toast", "name":index_name, "identity":format!("pg_toast.{index_name}")});
    [
        json!({"raw":{"oid":parent_oid,"reltoastrelid":toast_oid,"relname":"agent_instance_volume_leases","relkind":"r"},"namespace":"public","identity":parent_identity}),
        json!({"raw":{"oid":toast_oid,"reltoastrelid":"0","reltype":"0","relname":name,"relkind":"t"},"namespace":"pg_toast","identity":toast_identity,"toast_parent":parent_identity}),
        json!({"raw":{"oid":index_oid,"reltoastrelid":"0","reltype":"0","relname":index_name,"relkind":"i"},"namespace":"pg_toast","identity":index_identity,"toast_index_parent":parent_identity}),
        json!({"raw":{"indexrelid":index_oid,"indrelid":toast_oid},"identity":index_identity,"table":toast_identity}),
    ]
}

fn check(values: &[Value; 4]) {
    graph(&values[0], &values[1], &values[2], &values[3]);
}

#[test]
fn rewrite_graph_accepts_same_parent_replacement_and_independent_fresh_parent() {
    for values in [
        fixture("17000", "17009", "17010"),
        fixture("17000", "19009", "19010"),
        fixture("29000", "29009", "29010"),
    ] {
        check(&values);
    }
}

#[test]
fn rewrite_graph_denies_unrelated_parent_and_broken_physical_links() {
    let original = fixture("17000", "19009", "19010");
    for (record, field, drift) in [
        (
            0,
            "identity",
            json!({"type":"table", "schema":"public", "name":"other", "identity":"public.other"}),
        ),
        (
            1,
            "toast_parent",
            json!({"type":"table", "schema":"public", "name":"other", "identity":"public.other"}),
        ),
        (
            3,
            "table",
            json!({"type":"toast table", "schema":"pg_toast", "name":"pg_toast_99999", "identity":"pg_toast.pg_toast_99999"}),
        ),
    ] {
        let mut mutated = original.clone();
        mutated[record][field] = drift;
        assert!(catch_unwind(|| check(&mutated)).is_err(), "{field}");
    }
    for (record, field, drift) in [
        (0, "reltoastrelid", "19019"),
        (1, "relname", "pg_toast_99999"),
        (2, "relname", "pg_toast_99999_index"),
        (2, "reltype", "19020"),
        (3, "indexrelid", "19019"),
        (3, "indrelid", "19019"),
    ] {
        let mut mutated = original.clone();
        mutated[record]["raw"][field] = json!(drift);
        assert!(catch_unwind(|| check(&mutated)).is_err(), "{field}");
    }
}
