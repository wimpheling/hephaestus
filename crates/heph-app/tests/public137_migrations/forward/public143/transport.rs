use super::{Endpoint, Snapshot, TABLE, additions, indexed};
use std::collections::BTreeSet;
pub(super) fn constraints(before: &Snapshot, after: &Snapshot, added: &mut BTreeSet<Endpoint>) {
    let new = additions(before, after, "constraints");
    assert_eq!(new.len(), 40);
    let all = indexed(after, "relations");
    let parent = all
        .values()
        .find(|r| r.value["identity"]["identity"] == TABLE)
        .unwrap();
    let mut counts = std::collections::BTreeMap::new();
    for r in new {
        assert_eq!(r.value["raw"]["conrelid"], parent.value["raw"]["oid"]);
        *counts
            .entry(r.value["raw"]["contype"].as_str().unwrap())
            .or_insert(0) += 1;
        assert!(added.insert((
            "2606".to_owned(),
            r.value["raw"]["oid"].as_str().unwrap().to_owned(),
            0
        )));
    }
    assert_eq!(
        counts,
        std::collections::BTreeMap::from([("c", 15), ("f", 16), ("p", 1), ("t", 1), ("u", 7)])
    );
}
pub(super) fn triggers(before: &Snapshot, after: &Snapshot, added: &mut BTreeSet<Endpoint>) {
    let new = additions(before, after, "triggers");
    assert_eq!(new.len(), 66);
    let constraints = indexed(after, "constraints");
    let functions = indexed(after, "routines");
    let mut slots = BTreeSet::new();
    let mut explicit = BTreeSet::new();
    for r in new {
        let raw = &r.value["raw"];
        let id = raw["oid"].as_str().unwrap();
        let name = raw["tgname"].as_str().unwrap();
        if raw["tgisinternal"] == true {
            let fk = constraints[raw["tgconstraint"].as_str().unwrap()];
            foreign(raw, fk.value, id, name, &mut slots);
        } else {
            let (function, timing, deferred) = match name {
                "aaa_prepared_instance_terminal_guard143" => {
                    ("guard_prepared_instance_terminal143", 31, false)
                }
                "prepared_instance_terminal_commit143" => {
                    ("validate_prepared_instance_terminal143", 5, true)
                }
                other => panic!("unknown143 explicit hook {other}"),
            };
            assert!(explicit.insert(name));
            assert_eq!(r.value["relation"]["identity"], TABLE);
            let f = functions[raw["tgfoid"].as_str().unwrap()];
            assert_eq!(f.value["raw"]["proname"], function);
            assert_eq!(r.value["function"], f.value["identity"]);
            assert_eq!(raw["tgtype"], timing);
            assert_eq!(raw["tgdeferrable"], deferred);
            assert_eq!(raw["tginitdeferred"], deferred);
        }
        assert_eq!(raw["tgenabled"], "O");
        assert_eq!(raw["tgnargs"], 0);
        assert!(raw["tgqual"].is_null());
        assert!(added.insert(("2620".to_owned(), id.to_owned(), 0)));
    }
    assert_eq!(slots.len(), 64);
    assert_eq!(explicit.len(), 2);
}
pub(super) fn defaults(before: &Snapshot, after: &Snapshot, added: &mut BTreeSet<Endpoint>) {
    let new = additions(before, after, "defaults");
    assert_eq!(new.len(), 1);
    let record = new[0];
    assert_eq!(record.value["relation"]["identity"], TABLE);
    assert_eq!(record.value["raw"]["adnum"], 32);
    assert!(added.insert((
        "2604".to_owned(),
        record.value["raw"]["oid"].as_str().unwrap().to_owned(),
        0
    )));
}

fn foreign<'a>(
    raw: &'a serde_json::Value,
    fk: &serde_json::Value,
    id: &str,
    name: &str,
    slots: &mut BTreeSet<(&'a str, &'static str)>,
) {
    assert_eq!(fk["relation"]["identity"], TABLE);
    assert_eq!(fk["raw"]["contype"], "f");
    let (function, timing, check) = match raw["tgfoid"].as_str().unwrap() {
        "1644" => ("RI_FKey_check_ins", 5, true),
        "1645" => ("RI_FKey_check_upd", 17, true),
        "1654" => ("RI_FKey_noaction_del", 9, false),
        "1655" => ("RI_FKey_noaction_upd", 17, false),
        other => panic!("unknown143 RI function {other}"),
    };
    assert!(slots.insert((raw["tgconstraint"].as_str().unwrap(), function)));
    assert_eq!(raw["tgtype"], timing);
    assert_eq!(
        raw["tgrelid"],
        fk["raw"][if check { "conrelid" } else { "confrelid" }]
    );
    assert_eq!(
        raw["tgconstrrelid"],
        fk["raw"][if check { "confrelid" } else { "conrelid" }]
    );
    assert_eq!(raw["tgconstrindid"], fk["raw"]["conindid"]);
    assert_eq!(
        name,
        format!(
            "RI_ConstraintTrigger_{}_{id}",
            if check { 'c' } else { 'a' }
        )
    );
    assert_eq!(raw["tgdeferrable"], fk["raw"]["condeferrable"]);
    assert_eq!(raw["tginitdeferred"], fk["raw"]["condeferred"]);
}
