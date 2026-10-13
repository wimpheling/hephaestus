//! Finite terminal Source143 routine bodies, security metadata and physical identities.
use super::{Declared, Snapshot, inventory, metadata};
use serde_json::{Value, json};
use std::collections::BTreeSet;
#[path = "terminal143/entries.rs"]
mod entries;
use entries::ENTRIES;
// Source flags separately describe actual security attributes and addition identity.
type Entry = (
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static str,
    bool,
    bool,
    bool,
    bool,
);
pub(super) fn contains(name: &str) -> bool {
    ENTRIES.iter().any(|entry| entry.0 == name)
}
pub(super) fn replaces(name: &str) -> bool {
    ENTRIES.iter().any(|entry| entry.0 == name && !entry.7)
}
pub(super) fn assert_source(routine: &Value, declaration: &Declared) {
    let Some((name, types, language, volatility, security, app, worker, _)) =
        ENTRIES.iter().find(|entry| entry.0 == declaration.name)
    else {
        return;
    };
    assert_eq!(
        declaration
            .input_types
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        *types
    );
    let source = super::super::fixture::MIGRATIONS
        .iter()
        .find(|entry| entry.version == 143)
        .unwrap();
    let (_, tail) = source.sql.split_once(&format!("FUNCTION {name}(")).unwrap();
    let (_, tail) = tail.split_once(" AS $$").unwrap();
    assert_eq!(routine["raw"]["prosrc"], tail.split_once("$$;").unwrap().0);
    assert_eq!(routine["owner"], "postgres");
    assert_eq!(routine["raw"]["prokind"], "f");
    assert_eq!(routine["language"]["lanname"], *language);
    assert_eq!(routine["raw"]["provolatile"], *volatility);
    assert_eq!(routine["raw"]["prosecdef"], *security);
    assert_eq!(routine["raw"]["proparallel"], "u");
    assert_eq!(routine["raw"]["pronargdefaults"], 0);
    assert_eq!(
        routine["raw"]["proconfig"],
        json!([if *name == "recipe_command_guard" {
            "search_path=pg_catalog, public"
        } else {
            "search_path=pg_catalog, public, pg_temp"
        }])
    );
    assert!(!(*app && *worker));
    assert_eq!(
        routine["raw"]["proacl"],
        if *app {
            json!(["postgres=X/postgres", "hephaestus_app=X/postgres"])
        } else if *worker {
            json!(["postgres=X/postgres", "hephaestus_worker=X/postgres"])
        } else {
            json!(["postgres=X/postgres"])
        }
    );
}
pub(super) fn forward(before: &Snapshot, after: &Snapshot) -> BTreeSet<String> {
    let old = inventory::routines(before);
    let current = inventory::routines(after);
    let policy = super::policy();
    let declared = super::declarations(&policy);
    let added: BTreeSet<_> = ENTRIES
        .iter()
        .filter(|entry| entry.7)
        .map(|entry| {
            metadata::signature(
                entry.0,
                &entry.1.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
            )
        })
        .collect();
    assert_eq!(added.len(), 16);
    assert_eq!(
        current.keys().cloned().collect::<BTreeSet<_>>(),
        old.keys().cloned().chain(added.iter().cloned()).collect()
    );
    let mut replaced = 0;
    for (key, original) in &old {
        let following = current[key];
        let mut expected = (**original).clone();
        if replaces(metadata::text(&original["raw"]["proname"])) {
            assert_source(following, declared[key]);
            expected["raw"]["prosrc"] = following["raw"]["prosrc"].clone();
            expected["definition"] = following["definition"].clone();
            replaced += 1;
        }
        assert_eq!(
            expected, *following,
            "143 preserves old full routine identity/security/metadata except eleven sealed bodies"
        );
    }
    assert_eq!(replaced, 11);
    added
        .iter()
        .map(|key| {
            let routine = current[key];
            assert_source(routine, declared[key]);
            let function_oid = metadata::oid(&routine["raw"]["oid"]);
            assert!(
                old.values()
                    .all(|v| metadata::oid(&v["raw"]["oid"]) != function_oid)
            );
            function_oid.to_owned()
        })
        .collect()
}
