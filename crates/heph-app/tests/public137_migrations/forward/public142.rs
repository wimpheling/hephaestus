//! Exact141→142 raw identity preservation plus finite source additions.
use super::{
    Record, Snapshot, catalog_routines, consume, context, counts, exact, fields, indexed, records,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

type Endpoint = (String, String, i64);
const REMOVAL: &str = "public.agent_instance_removal_requests";
const RECEIPT: &str = "recipe_deployment_commands_receipt_lifecycle_check";
const FK: &str = "instance_removal_prepared_resume_command142_fkey";
const NON_NIL: &str = "instance_removal_prepared_resume_command142_non_nil";
const COMMIT: &str = "prepared_resume_origin_commit142";

pub(super) fn assert142(before: &Snapshot, after: &Snapshot) {
    context(before, after);
    let mut added: BTreeSet<Endpoint> = catalog_routines::assert_forward142(before, after)
        .into_iter()
        .map(|id| ("1255".to_owned(), id, 0))
        .collect();
    routine_text(before, after);
    relations(before, after);
    columns(before, after, &mut added);
    let retired = constraints(before, after, &mut added);
    triggers(before, after, &mut added);
    for (domain, original) in before.domains() {
        match domain.as_str() {
            "routines" | "relations" | "columns" | "constraints" | "triggers" => (),
            "dependencies" | "shared_dependencies" => {
                edges(before, after, domain, &added, &retired);
            }
            _ => assert_eq!(
                original,
                &after.domains()[domain],
                "unchanged141→142 {domain}"
            ),
        }
    }
}

fn routine_text(before: &Snapshot, after: &Snapshot) {
    let current = indexed(after, "routines");
    let mut replaced = 0;
    for (id, original) in indexed(before, "routines") {
        let following = current[id];
        if matches!(
            original.value["raw"]["proname"].as_str().unwrap(),
            "remove_resume_context118"
                | "queued_processed_closure140"
                | "queued_cancel_closure140"
                | "queued_removal_origin140"
                | "guard_remove_resume_admission118"
                | "load_existing_instance_closure118"
                | "recipe_command_guard"
                | "validate_ordinary_queued_cancel_commit140"
        ) {
            let mut expected = fields(original);
            let actual = fields(following);
            for field in ["raw.prosrc", "definition"] {
                assert!(expected.insert(field.to_owned(), actual[field]).is_some());
            }
            assert_eq!(
                expected, actual,
                "142 only replaces sealed bodies, preserving every identity/ACL field"
            );
            replaced += 1;
        } else {
            exact(original, following);
        }
    }
    assert_eq!(replaced, 8);
}

fn relations(before: &Snapshot, after: &Snapshot) {
    let old = indexed(before, "relations");
    let current = indexed(after, "relations");
    assert!(old.keys().eq(current.keys()));
    let mut changed = 0;
    for (id, original) in old {
        let following = current[id];
        if original.value["identity"]["identity"] == REMOVAL {
            assert_eq!(original.value["raw"]["relnatts"], 8);
            assert_eq!(following.value["raw"]["relnatts"], 9);
            assert_eq!(original.value["raw"]["relchecks"], 4);
            assert_eq!(following.value["raw"]["relchecks"], 5);
            let mut expected = fields(original);
            let actual = fields(following);
            for field in ["raw.relnatts", "raw.relchecks"] {
                assert!(expected.insert(field.to_owned(), actual[field]).is_some());
            }
            assert_eq!(
                expected, actual,
                "142 removal table retains every other raw/text field"
            );
            changed += 1;
        } else {
            exact(original, following);
        }
    }
    assert_eq!(changed, 1);
}

fn attributes(snapshot: &Snapshot) -> BTreeMap<(String, i64), Record<'_>> {
    let mut result = BTreeMap::new();
    for record in records(snapshot, "columns") {
        let raw = &record.value["raw"];
        let key = (
            raw["attrelid"].as_str().unwrap().to_owned(),
            raw["attnum"].as_i64().unwrap(),
        );
        assert!(result.insert(key, record).is_none());
    }
    result
}

fn columns(before: &Snapshot, after: &Snapshot, added: &mut BTreeSet<Endpoint>) {
    let old = attributes(before);
    let current = attributes(after);
    for (key, original) in &old {
        exact(*original, current[key]);
    }
    let additions: Vec<_> = current
        .iter()
        .filter(|(key, _)| !old.contains_key(*key))
        .collect();
    assert_eq!(additions.len(), 1);
    let (key, record) = additions[0];
    let raw = &record.value["raw"];
    assert_eq!(record.value["relation"]["identity"], REMOVAL);
    assert_eq!(key.1, 9);
    assert_eq!(raw["attname"], "prepared_resume_command_id");
    assert_eq!(raw["atttypid"], "2950");
    assert_eq!(record.value["formatted_type"], "uuid");
    for field in ["attnotnull", "atthasdef", "atthasmissing", "attisdropped"] {
        assert_eq!(raw[field], false);
    }
    assert_eq!(raw["attidentity"], "");
    assert_eq!(raw["attgenerated"], "");
    assert!(raw["attacl"].is_null());
    assert!(added.insert(("1259".to_owned(), key.0.clone(), 9)));
}

fn constraints(
    before: &Snapshot,
    after: &Snapshot,
    added: &mut BTreeSet<Endpoint>,
) -> BTreeSet<Endpoint> {
    let old = indexed(before, "constraints");
    let current = indexed(after, "constraints");
    let mut retired = BTreeSet::new();
    for (id, original) in &old {
        if original.value["raw"]["conname"] == RECEIPT {
            assert_eq!(
                original.value["relation"]["identity"],
                "public.recipe_deployment_commands"
            );
            assert!(!current.contains_key(id));
            assert!(retired.insert(("2606".to_owned(), (*id).to_owned(), 0)));
        } else {
            exact(*original, current[id]);
        }
    }
    assert_eq!(retired.len(), 1);
    let additions: Vec<_> = current
        .iter()
        .filter(|(id, _)| !old.contains_key(*id))
        .collect();
    assert_eq!(additions.len(), 4);
    for (name, table, kind) in [
        (FK, REMOVAL, "f"),
        (NON_NIL, REMOVAL, "c"),
        (COMMIT, REMOVAL, "t"),
        (RECEIPT, "public.recipe_deployment_commands", "c"),
    ] {
        let matches: Vec<_> = additions
            .iter()
            .filter(|(_, r)| r.value["raw"]["conname"] == name)
            .collect();
        assert_eq!(matches.len(), 1);
        let (id, record) = *matches[0];
        assert_eq!(record.value["relation"]["identity"], table);
        assert_eq!(record.value["raw"]["contype"], kind);
        if name == RECEIPT {
            let original = old
                .values()
                .find(|r| r.value["raw"]["conname"] == RECEIPT)
                .unwrap();
            let mut expected = fields(*original);
            let actual = fields(*record);
            for field in ["raw.oid", "raw.conbin", "definition"] {
                assert!(expected.insert(field.to_owned(), actual[field]).is_some());
            }
            assert_eq!(
                expected, actual,
                "only142 rebuilds the exact receipt CHECK expression/OID"
            );
            assert_eq!(
                record.value["definition"],
                "CHECK ((receipt_lifecycle = ANY (ARRAY['installing'::text, 'removing'::text, 'removed'::text, 'recovery_required'::text])))"
            );
        }
        assert!(added.insert(("2606".to_owned(), (*id).to_owned(), 0)));
    }
    retired
}

fn triggers(before: &Snapshot, after: &Snapshot, added: &mut BTreeSet<Endpoint>) {
    let old = indexed(before, "triggers");
    let current = indexed(after, "triggers");
    for (id, r) in &old {
        exact(*r, current[id]);
    }
    let additions: Vec<_> = current
        .iter()
        .filter(|(id, _)| !old.contains_key(*id))
        .collect();
    assert_eq!(additions.len(), 6);
    let constraints = indexed(after, "constraints");
    let foreign = constraints
        .values()
        .find(|r| r.value["raw"]["conname"] == FK)
        .unwrap();
    let mut slots = BTreeSet::new();
    let mut explicit = BTreeSet::new();
    for (id, record) in additions {
        let raw = &record.value["raw"];
        let name = raw["tgname"].as_str().unwrap();
        if raw["tgconstraint"] == foreign.value["raw"]["oid"] {
            foreign_trigger(raw, foreign, name, id, &mut slots);
        } else {
            let (function, event, deferred) = match name {
                "aaa_prepared_resume_origin142" => ("guard_prepared_resume_origin142", 7, false),
                COMMIT => ("validate_prepared_resume_origin142", 5, true),
                other => panic!("unknown142 trigger {other}"),
            };
            assert!(explicit.insert(name));
            assert_eq!(record.value["relation"]["identity"], REMOVAL);
            let functions = indexed(after, "routines");
            let routine = functions[raw["tgfoid"].as_str().unwrap()];
            assert_eq!(routine.value["raw"]["proname"], function);
            assert_eq!(record.value["function"], routine.value["identity"]);
            assert_eq!(raw["tgtype"], event);
            assert_eq!(raw["tgisinternal"], false);
            assert_eq!(raw["tgdeferrable"], deferred);
            assert_eq!(raw["tginitdeferred"], deferred);
        }
        assert_eq!(raw["tgenabled"], "O");
        assert_eq!(raw["tgnargs"], 0);
        assert!(raw["tgqual"].is_null());
        assert!(added.insert(("2620".to_owned(), (*id).to_owned(), 0)));
    }
    assert_eq!(slots.len(), 4);
    assert_eq!(explicit.len(), 2);
}

fn endpoint(raw: &Value, reference: bool) -> Endpoint {
    let (class, id, sub) = if reference {
        ("refclassid", "refobjid", "refobjsubid")
    } else {
        ("classid", "objid", "objsubid")
    };
    (
        raw[class].as_str().unwrap().to_owned(),
        raw[id].as_str().unwrap().to_owned(),
        raw.get(sub).map_or(0, |v| v.as_i64().unwrap()),
    )
}

fn edges(
    before: &Snapshot,
    after: &Snapshot,
    domain: &str,
    added: &BTreeSet<Endpoint>,
    retired: &BTreeSet<Endpoint>,
) {
    let current = records(after, domain);
    let mut remaining = counts(&current);
    for record in records(before, domain) {
        let raw = &record.value["raw"];
        if retired.contains(&endpoint(raw, false))
            || (domain == "dependencies" && retired.contains(&endpoint(raw, true)))
        {
            assert_eq!(
                remaining.get(record.text).copied().unwrap_or(0),
                0,
                "old rebuilt CHECK edges cannot survive"
            );
        } else {
            consume(&mut remaining, record.text);
        }
    }
    for record in current {
        let count = remaining.get_mut(record.text).unwrap();
        if *count == 0 {
            continue;
        }
        *count -= 1;
        let raw = &record.value["raw"];
        assert!(
            added.contains(&endpoint(raw, false))
                || (domain == "dependencies" && added.contains(&endpoint(raw, true))),
            "142 edge must touch an exact typed source-added endpoint"
        );
    }
}

fn foreign_trigger(
    raw: &Value,
    foreign: &Record<'_>,
    name: &str,
    id: &str,
    slots: &mut BTreeSet<&'static str>,
) -> &'static str {
    assert_eq!(raw["tgisinternal"], true);
    let (function, event, check) = match raw["tgfoid"].as_str().unwrap() {
        "1644" => ("RI_FKey_check_ins", 5, true),
        "1645" => ("RI_FKey_check_upd", 17, true),
        "1654" => ("RI_FKey_noaction_del", 9, false),
        "1655" => ("RI_FKey_noaction_upd", 17, false),
        other => panic!("unknown142 RI function {other}"),
    };
    assert!(slots.insert(function));
    assert_eq!(raw["tgtype"], event);
    assert_eq!(
        raw["tgrelid"],
        foreign.value["raw"][if check { "conrelid" } else { "confrelid" }]
    );
    assert_eq!(
        raw["tgconstrrelid"],
        foreign.value["raw"][if check { "confrelid" } else { "conrelid" }]
    );
    assert_eq!(raw["tgconstrindid"], foreign.value["raw"]["conindid"]);
    assert_eq!(
        name,
        format!(
            "RI_ConstraintTrigger_{}_{id}",
            if check { 'c' } else { 'a' }
        )
    );
    assert_eq!(raw["tgdeferrable"], false);
    assert_eq!(raw["tginitdeferred"], false);
    function
}
