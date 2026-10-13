//! Fragment substitutions exercise production helpers, not fabricated Snapshots.
use super::{Inventory, Record, assert_inventory, assert_semantics, ast, semantic, source};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const FUNCTION: &str = "{FUNCEXPR :funcid 20000 :funcresulttype 16 :funcretset false :funcvariadic false :funcformat 0 :funccollid 0 :inputcollid 0 :args ({CONST :consttype 25 :consttypmod -1 :constcollid 100 :constlen -1 :constbyval false :constisnull false :location 41 :constvalue 8 [ 32 78 0 0 0 0 0 0 ]}) :location 13}";
const RTE: &str = "{RANGETBLENTRY :alias <> :eref {ALIAS :aliasname t :colnames (id)} :rtekind 0 :relid 20001 :inh true :relkind r :rellockmode 1 :perminfoindex 1 :tablesample <> :lateral false :inFromCl true :securityQuals <>}";
const PERMISSIONS: &str = "{RTEPERMISSIONINFO :relid 20001 :inh true :requiredPerms 2 :checkAsUser 0 :selectedCols (b 8) :insertedCols (b) :updatedCols (b)}";

fn bindings() -> (BTreeMap<String, Value>, BTreeMap<String, Value>) {
    (
        BTreeMap::from([(
            "20000".into(),
            json!("public.check_permission(text,text,text,text,text)"),
        )]),
        BTreeMap::from([("20001".into(), json!("public.target"))]),
    )
}

#[test]
fn embedded_sources_admit_exactly_sixty_three_policies() {
    let declarations = source::additions();
    assert_eq!(declarations.len(), 63);
    let row = &declarations["public.run_vm_plan_admissions.run_vm_admission_actor_insert"];
    assert_eq!(row.command, "a");
    assert!(row.permissive);
    assert_eq!(row.roles, BTreeSet::from(["hephaestus_app".into()]));
}

#[test]
fn declarations_skip_ghosts_and_bind_complete_header() {
    let sql = r#"/* CREATE POLICY ghost ON x; /* nested */ */
        CREATE FUNCTION f() RETURNS void LANGUAGE sql AS $$SELECT 'CREATE POLICY ghost ON x;'$$;
        SELECT 'CREATE POLICY ghost ON x;', "CREATE POLICY ghost";
        CREATE POLICY actual ON public.t AS RESTRICTIVE FOR UPDATE TO hephaestus_worker, PUBLIC
            USING (true) WITH CHECK (true);"#;
    let rows = source::declarations(sql);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows["public.t.actual"],
        source::Declaration {
            command: "w",
            permissive: false,
            roles: BTreeSet::from(["hephaestus_worker".into(), "PUBLIC".into()])
        }
    );
}

#[test]
fn replacements_duplicates_unknown_headers_and_foreign_schema_deny() {
    for sql in [
        "ALTER POLICY a ON t USING(true);",
        "DROP POLICY a ON t;",
        "CREATE POLICY a ON t USING(true); CREATE POLICY a ON t USING(true);",
        "CREATE POLICY a ON foreign_schema.t USING(true);",
        "CREATE POLICY a ON t AS strange USING(true);",
        "CREATE POLICY a ON t FOR strange USING(true);",
        "CREATE POLICY \"a\" ON t USING(true);",
        "CREATE POLICY a ON t TO PUBLIC, PUBLIC USING(true);",
    ] {
        assert!(std::panic::catch_unwind(|| source::declarations(sql)).is_err());
    }
}

#[test]
fn only_four_typed_slots_change_and_all_bytes_reconstruct() {
    let target = "{TARGETENTRY :expr {CONST :consttype 23 :consttypmod -1 :constcollid 0 :constlen 4 :constbyval true :constisnull false :location 70 :constvalue 4 [ 1 0 0 0 ]} :resno 1 :resname id :ressortgroupref 0 :resorigtbl 20001 :resorigcol 1 :resjunk false}";
    let text = format!(
        "{{BOOLEXPR :boolop and :args ({FUNCTION} {RTE} {PERMISSIONS} {target}) :location 17}}"
    );
    let (functions, classes) = bindings();
    let result = ast::normalized(&text, &functions, &classes);
    let mut rebuilt = String::new();
    let mut slots = Vec::new();
    for part in result.as_array().unwrap() {
        if let Some(fragment) = part.as_str() {
            rebuilt.push_str(fragment);
        } else {
            let slot = part["slot"].as_str().unwrap();
            rebuilt.push_str(if slot == "FUNCEXPR.:funcid" {
                "20000"
            } else {
                "20001"
            });
            slots.push(slot);
        }
    }
    assert_eq!(
        slots,
        [
            "FUNCEXPR.:funcid",
            "RANGETBLENTRY.:relid",
            "RTEPERMISSIONINFO.:relid",
            "TARGETENTRY.:resorigtbl"
        ]
    );
    assert_eq!(rebuilt, text);
}

#[test]
fn constants_types_collations_locations_and_permission_bits_remain_exact() {
    let (functions, classes) = bindings();
    for text in [FUNCTION, RTE, PERMISSIONS] {
        let expected = ast::normalized(text, &functions, &classes);
        let changed = match text {
            FUNCTION => text.replace("32 78 0", "33 78 0"),
            RTE => text.replace(":rellockmode 1", ":rellockmode 3"),
            _ => text.replace(":selectedCols (b 8)", ":selectedCols (b 9)"),
        };
        assert_ne!(expected, ast::normalized(&changed, &functions, &classes));
    }
    for (from, to) in [
        (":funcresulttype 16", ":funcresulttype 90000"),
        (":inputcollid 0", ":inputcollid 80000"),
        (":location 13", ":location 14"),
        (":consttype 25", ":consttype 70000"),
    ] {
        assert_ne!(
            ast::normalized(FUNCTION, &functions, &classes),
            ast::normalized(&FUNCTION.replace(from, to), &functions, &classes)
        );
    }
    assert_ne!(
        ast::normalized(PERMISSIONS, &functions, &classes),
        ast::normalized(
            &PERMISSIONS.replace(":checkAsUser 0", ":checkAsUser 20000"),
            &functions,
            &classes
        )
    );
}

#[test]
fn unknown_nodes_missing_duplicate_fields_and_unqualified_slots_deny() {
    let (functions, classes) = bindings();
    for text in [
        "{UNKNOWN :funcid 20000}".into(),
        FUNCTION.replace(" :funcresulttype 16", ""),
        FUNCTION.replace(":funcid 20000", ":funcid 20000 :funcid 20000"),
        FUNCTION.replace(":funcid 20000", ":funcid 20002"),
        RTE.replace(":relid 20001", ":relid 20002"),
        RTE.replace(":rtekind 0", ":rtekind 9"),
        PERMISSIONS.replace(" :updatedCols (b)", ""),
        PERMISSIONS.replace(":requiredPerms 2", ":requiredPerms 2 :newPermissionField 4"),
    ] {
        assert!(std::panic::catch_unwind(|| ast::normalized(&text, &functions, &classes)).is_err());
    }
}

#[test]
fn joins_and_zero_target_origin_stay_complete() {
    let text = "{RANGETBLENTRY :alias <> :eref {ALIAS :aliasname t :colnames (id)} :rtekind 2 :jointype 0 :joinmergedcols 0 :joinaliasvars <> :joinleftcols (i 1) :joinrightcols (i 1) :join_using_alias <> :lateral false :inFromCl true :securityQuals <>}";
    assert_eq!(
        ast::normalized(text, &BTreeMap::new(), &BTreeMap::new()),
        json!([text])
    );
    let target = "{TARGETENTRY :expr <> :resno 1 :resname value :ressortgroupref 0 :resorigtbl 0 :resorigcol 0 :resjunk false}";
    assert_eq!(
        ast::normalized(target, &BTreeMap::new(), &BTreeMap::new()),
        json!([target])
    );
}

#[test]
fn unknown_inventory_is_not_admitted_by_target() {
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
    for expected in [
        BTreeSet::new(),
        BTreeSet::from(["public.t.a".into(), "public.t.extra".into()]),
    ] {
        assert!(std::panic::catch_unwind(|| assert_inventory(&rows, &expected)).is_err());
    }
}

#[test]
fn complete_semantics_reject_role_command_mode_and_deparser_substitutions() {
    let value = json!({"raw": {"polcmd": "r", "polpermissive": true, "polroles": ["0"],
        "polqual": "complete AST", "polwithcheck": null}, "roles": [{"oid": "0", "name": "PUBLIC"}],
        "using": "full USING deparser", "check": null});
    let text = value.to_string();
    let expected = semantic(Record {
        value: &value,
        text: &text,
    });
    for (field, substitution) in [
        ("polcmd", json!("a")),
        ("polpermissive", json!(false)),
        ("polroles", json!(["20000"])),
        ("polqual", json!("another AST")),
        ("polwithcheck", json!("true")),
    ] {
        let mut changed = value.clone();
        changed["raw"][field] = substitution;
        let changed_text = changed.to_string();
        assert!(
            std::panic::catch_unwind(|| assert_semantics(
                &semantic(Record {
                    value: &changed,
                    text: &changed_text
                }),
                &expected
            ))
            .is_err()
        );
    }
    for (field, substitution) in [
        ("roles", json!([{"oid": "0", "name": "other"}])),
        ("using", json!("true")),
        ("check", json!("true")),
    ] {
        let mut changed = value.clone();
        changed[field] = substitution;
        let changed_text = changed.to_string();
        assert!(
            std::panic::catch_unwind(|| assert_semantics(
                &semantic(Record {
                    value: &changed,
                    text: &changed_text
                }),
                &expected
            ))
            .is_err()
        );
    }
}

#[test]
fn public_role_requires_canonical_zero_oid_and_exact_name() {
    super::assert_public_role(&json!("0"), &json!("PUBLIC"));
    for (oid, name) in [
        ("1", "PUBLIC"),
        ("00", "PUBLIC"),
        ("0", "public"),
        ("0", "worker"),
    ] {
        assert!(
            std::panic::catch_unwind(|| super::assert_public_role(&json!(oid), &json!(name)))
                .is_err()
        );
    }
}

#[test]
fn current_user_binding_requires_qualified_migration_and_capture_owner() {
    let context = json!({"session_user": "postgres", "current_user": "postgres", "role": "none"});
    let actual = BTreeSet::from(["postgres".into()]);
    let declared = BTreeSet::from(["current_user".into()]);
    super::assert_source_roles(&context, &actual, &declared);
    for (field, wrong) in [
        ("session_user", json!("hephaestus_app")),
        ("current_user", json!("hephaestus_worker")),
        ("current_user", json!(null)),
        ("role", json!("hephaestus_worker")),
        ("role", json!(null)),
    ] {
        let mut changed = context.clone();
        changed[field] = wrong;
        assert!(
            std::panic::catch_unwind(|| super::assert_source_roles(&changed, &actual, &declared))
                .is_err()
        );
    }
    assert!(
        std::panic::catch_unwind(|| super::assert_source_roles(&json!({}), &actual, &declared))
            .is_err()
    );
}

#[test]
fn binding_changes_only_exact_source_current_user_and_preserves_public() {
    let context = json!({"session_user": "postgres", "current_user": "postgres", "role": "none"});
    let actual = BTreeSet::from(["postgres".into(), "PUBLIC".into()]);
    let declared = BTreeSet::from(["current_user".into(), "PUBLIC".into()]);
    super::assert_source_roles(&context, &actual, &declared);
    super::assert_public_role(&json!("0"), &json!("PUBLIC"));
    for wrong in [
        BTreeSet::from(["current_user".into(), "PUBLIC".into()]),
        BTreeSet::from(["hephaestus_app".into(), "PUBLIC".into()]),
        BTreeSet::from(["postgres".into(), "public".into()]),
        BTreeSet::from(["postgres".into()]),
    ] {
        assert!(
            std::panic::catch_unwind(|| super::assert_source_roles(&context, &wrong, &declared))
                .is_err()
        );
    }
    for wrong in ["CURRENT_USER", "current_user_extra", "session_user"] {
        let declared = BTreeSet::from([wrong.into(), "PUBLIC".into()]);
        assert!(
            std::panic::catch_unwind(|| super::assert_source_roles(&context, &actual, &declared))
                .is_err()
        );
    }
}

#[test]
fn source_parser_keeps_pseudo_role_token_and_rejects_quoted_roles() {
    for sql in [
        "CREATE POLICY a ON t TO CURRENT_USER USING(true);",
        "CREATE POLICY a ON t TO current_user USING(true);",
    ] {
        assert_eq!(
            source::declarations(sql)["public.t.a"].roles,
            BTreeSet::from(["current_user".into()])
        );
    }
    assert_eq!(source::declarations("CREATE POLICY a ON t TO current_user_extra USING(true);")["public.t.a"].roles, BTreeSet::from(["current_user_extra".into()]));
    assert!(
        std::panic::catch_unwind(|| source::declarations(
            r#"CREATE POLICY a ON t TO "current_user" USING(true);"#
        ))
        .is_err()
    );
}
