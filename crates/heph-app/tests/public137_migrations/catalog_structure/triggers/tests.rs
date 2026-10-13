//! Helper substitutions are not fabricated positive catalog/database snapshots.
use super::{Inventory, Record, assert_inventory, assert_semantics, ri, semantic, source};
use serde_json::json;
use std::collections::BTreeSet;

#[test]
fn embedded_source_has_exact_declared_additions() {
    let declarations = source::additions();
    assert_eq!(declarations.len(), 122);
    assert_eq!(
        declarations
            .values()
            .filter(|entry| entry.constraint)
            .count(),
        27
    );
    assert_eq!(
        declarations["public.run_cleanup_targets.run_cleanup_target_application_event"].function,
        "capture_direct_application_event"
    );
}

#[test]
fn declaration_scanner_skips_bodies_quotes_and_nested_comments() {
    let sql = r#"
        /* CREATE TRIGGER fake ON x; /* nested */ */
        CREATE FUNCTION f() RETURNS trigger LANGUAGE plpgsql AS $body$
            BEGIN RAISE NOTICE 'CREATE TRIGGER ghost ON y;'; END $body$;
        SELECT 'CREATE TRIGGER string ON z;', "CREATE TRIGGER identifier";
        -- CREATE TRIGGER line ON z;
        CREATE CONSTRAINT TRIGGER actual AFTER INSERT ON public.target
          DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
          EXECUTE FUNCTION public.hook('value; CREATE TRIGGER argv ON z;');
    "#;
    let rows = source::declarations(sql);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows["public.target.actual"],
        source::Declaration {
            function: "hook".into(),
            constraint: true
        }
    );
}

#[test]
fn duplicate_foreign_or_quoted_declarations_deny() {
    for sql in [
        "CREATE TRIGGER a AFTER INSERT ON t EXECUTE FUNCTION f(); CREATE TRIGGER a AFTER INSERT ON t EXECUTE FUNCTION f();",
        "CREATE TRIGGER a AFTER INSERT ON foreign_schema.t EXECUTE FUNCTION f();",
        "CREATE TRIGGER \"a\" AFTER INSERT ON t EXECUTE FUNCTION f();",
    ] {
        assert!(std::panic::catch_unwind(|| source::declarations(sql)).is_err());
    }
}

#[test]
fn generated_name_must_use_own_oid_and_exact_role_prefix() {
    let raw = json!({"oid": "20000", "tgname": "RI_ConstraintTrigger_c_20000"});
    assert_eq!(ri::name(&raw, "c_insert"), "RI_ConstraintTrigger_c_20000");
    for (field, value) in [
        ("oid", "20001"),
        ("tgname", "RI_ConstraintTrigger_c_9"),
        ("tgname", "arbitrary_20000"),
    ] {
        let mut changed = raw.clone();
        changed[field] = json!(value);
        assert!(std::panic::catch_unwind(|| ri::name(&changed, "c_insert")).is_err());
    }
    assert!(std::panic::catch_unwind(|| ri::name(&raw, "a_delete")).is_err());
}

#[test]
fn ri_role_rejects_rebound_table_index_function_and_deferral() {
    let constraint = json!({"conrelid": "20000", "confrelid": "20001", "conindid": "20002", "confdeltype": "a", "confupdtype": "c", "condeferrable": true, "condeferred": true});
    let raw = json!({"tgtype": 5, "tgfoid": "1644", "tgrelid": "20000", "tgconstrrelid": "20001", "tgconstrindid": "20002", "tgdeferrable": true, "tginitdeferred": true, "tgnargs": 0, "tgqual": null});
    assert_eq!(
        ri::role(&raw, &constraint),
        ("c_insert", "RI_FKey_check_ins")
    );
    for (field, value) in [
        ("tgtype", json!(9)),
        ("tgfoid", json!("1654")),
        ("tgrelid", json!("20001")),
        ("tgconstrrelid", json!("20000")),
        ("tgconstrindid", json!("20003")),
        ("tgdeferrable", json!(false)),
        ("tginitdeferred", json!(false)),
        ("tgnargs", json!(1)),
        ("tgqual", json!("{CONST :constvalue 20000}")),
    ] {
        let mut changed = raw.clone();
        changed[field] = value;
        assert!(std::panic::catch_unwind(|| ri::role(&changed, &constraint)).is_err());
    }
}

#[test]
fn ri_definition_maps_only_verified_first_name() {
    let definition = "CREATE CONSTRAINT TRIGGER \"RI_ConstraintTrigger_c_20000\" AFTER INSERT ON public.t FROM public.parent DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION \"RI_FKey_check_ins\"()";
    let value = json!({"definition": definition});
    let text = value.to_string();
    let mapped = ri::definition(
        Record {
            value: &value,
            text: &text,
        },
        "RI_ConstraintTrigger_c_20000",
        "qualified",
    );
    assert_eq!(
        mapped,
        format!(
            "CREATE CONSTRAINT TRIGGER qualified {}",
            definition.split_once("\" ").unwrap().1
        )
    );
    assert!(
        std::panic::catch_unwind(|| ri::definition(
            Record {
                value: &value,
                text: &text
            },
            "RI_ConstraintTrigger_c_20001",
            "qualified"
        ))
        .is_err()
    );
}

#[test]
fn full_trigger_inventory_rejects_missing_and_extra_records() {
    let value = json!({});
    let text = value.to_string();
    let rows = Inventory::from([(
        "public.t.a".into(),
        Record {
            value: &value,
            text: &text,
        },
    )]);
    assert_inventory(&rows, &BTreeSet::from(["public.t.a".into()]));
    for wrong in [
        BTreeSet::new(),
        BTreeSet::from(["public.t.a".into(), "public.t.extra".into()]),
    ] {
        assert!(std::panic::catch_unwind(|| assert_inventory(&rows, &wrong)).is_err());
    }
}

#[test]
fn canonical_comparison_keeps_flags_args_order_when_and_definition() {
    let value = json!({"raw": {"tgenabled": "O", "tgtype": 5, "tgargs": "\\x72656369706500", "tgnargs": 1, "tgattr": [2, 1], "tgqual": "{CONST :constvalue 20000}", "tgdeferrable": true, "tginitdeferred": false, "tgoldtable": null, "tgnewtable": null}, "definition": "literal complete definition"});
    let text = value.to_string();
    let expected = semantic(Record {
        value: &value,
        text: &text,
    });
    for (field, changed_value) in [
        ("tgenabled", json!("D")),
        ("tgtype", json!(17)),
        ("tgargs", json!("\\x")),
        ("tgnargs", json!(0)),
        ("tgattr", json!([1, 2])),
        ("tgqual", json!("{CONST :constvalue 20001}")),
        ("tgdeferrable", json!(false)),
        ("tginitdeferred", json!(true)),
        ("tgoldtable", json!("old_rows")),
        ("tgnewtable", json!("new_rows")),
    ] {
        let mut changed = value.clone();
        changed["raw"][field] = changed_value;
        let changed_text = changed.to_string();
        let actual = semantic(Record {
            value: &changed,
            text: &changed_text,
        });
        assert!(std::panic::catch_unwind(|| assert_semantics(&actual, &expected)).is_err());
    }
    let mut changed = value;
    changed["definition"] = json!("different definition");
    let changed_text = changed.to_string();
    let actual = semantic(Record {
        value: &changed,
        text: &changed_text,
    });
    assert!(std::panic::catch_unwind(|| assert_semantics(&actual, &expected)).is_err());
}

#[test]
fn ri_function_witness_cannot_be_public_or_rebound() {
    let helper = json!({"type": "function", "schema": "pg_catalog", "name": null, "identity": "pg_catalog.\"RI_FKey_check_ins\"()"});
    ri::function(&helper, "RI_FKey_check_ins");
    for field in ["type", "schema", "name", "identity"] {
        let mut changed = helper.clone();
        changed[field] = json!("forged");
        assert!(std::panic::catch_unwind(|| ri::function(&changed, "RI_FKey_check_ins")).is_err());
    }
}
