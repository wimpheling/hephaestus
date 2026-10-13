//! Routine-only upgrade oracle. The caller must independently qualify all three
//! real journals/source/native inputs: 107 before, upgraded 143, and fresh 143.
//! These assertions neither bind journal phases nor certify other catalog domains.
use super::catalog::Snapshot;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[path = "catalog_routines/inventory.rs"]
mod inventory;
#[path = "catalog_routines/metadata.rs"]
mod metadata;
#[path = "catalog_routines/preclosure142.rs"]
mod preclosure142;
#[path = "catalog_routines/queued_cancel.rs"]
mod queued_cancel;
#[path = "catalog_routines/terminal143.rs"]
mod terminal143;
use queued_cancel::assert_queued_cancel140;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    source_sha256: BTreeMap<String, String>,
    native_sha256: BTreeMap<String, String>,
    declared: Vec<Declared>,
    native: Vec<Native>,
}

// These independent source-policy flags describe four separate permitted deltas.
#[allow(clippy::struct_excessive_bools)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Declared {
    name: String,
    input_types: Vec<String>,
    body_sha256: String,
    replaced: bool,
    config_extended: bool,
    public_revoked: bool,
    hardened: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Native {
    name: String,
    input_types: Vec<String>,
    symbol: String,
    strict: bool,
    volatility: String,
}

pub fn assert_routines(before: &Snapshot, after: &Snapshot, reference: &Snapshot) {
    inventory::context(before, after, reference);
    let policy = policy();
    let old = inventory::routines(before);
    let upgraded = inventory::routines(after);
    let fresh = inventory::routines(reference);
    let declared = declarations(&policy);
    let native: BTreeSet<_> = policy
        .native
        .iter()
        .map(|entry| metadata::signature(&entry.name, &entry.input_types))
        .collect();
    assert_eq!(native.len(), policy.native.len());
    let previously_native = inventory::native_members(before, &native);
    assert!(inventory::native_members(after, &native));
    assert!(inventory::native_members(reference, &native));
    assert!(declared.keys().all(|key| !native.contains(key)));
    assert_eq!(
        upgraded.keys().collect::<Vec<_>>(),
        fresh.keys().collect::<Vec<_>>()
    );
    let expected_new: BTreeSet<_> = declared
        .iter()
        .filter(|(_, entry)| !entry.replaced)
        .map(|(key, _)| key.clone())
        .chain(if previously_native {
            BTreeSet::new()
        } else {
            native.clone()
        })
        .collect();
    let actual_new: BTreeSet<_> = upgraded
        .keys()
        .filter(|key| !old.contains_key(*key))
        .cloned()
        .collect();
    assert_eq!(
        actual_new, expected_new,
        "only independently declared source/native routines may be added"
    );
    for (key, old_routine) in &old {
        let current = upgraded
            .get(key)
            .expect("every original routine must survive");
        assert_old(key, old_routine, current, declared.get(key).copied());
    }
    for (key, entry) in &declared {
        assert_eq!(
            old.contains_key(key),
            entry.replaced,
            "source replacement classification"
        );
        let current = upgraded.get(key).expect("missing source-declared routine");
        assert_source(current, entry);
        assert_reference(after, reference, current, fresh[key]);
    }
    for entry in &policy.native {
        let key = metadata::signature(&entry.name, &entry.input_types);
        let current = upgraded[&key];
        assert_eq!(current["language"]["lanname"], "c");
        assert_eq!(current["raw"]["prosrc"], entry.symbol);
        assert_eq!(current["raw"]["probin"], "$libdir/pgcrypto");
        assert_eq!(current["raw"]["proisstrict"], entry.strict);
        assert_eq!(current["raw"]["provolatile"], entry.volatility);
        assert_eq!(current["raw"]["proparallel"], "s");
        assert_reference(after, reference, current, fresh[&key]);
    }
    // Other old routines must independently agree with the fresh canonical head.
    for key in old
        .keys()
        .filter(|key| !declared.contains_key(*key) && !native.contains(*key))
    {
        assert_reference(after, reference, upgraded[key], fresh[key]);
    }
}

fn policy() -> Policy {
    let policy: Policy =
        serde_json::from_str(include_str!("catalog_routines/policy.json")).unwrap();
    assert_eq!(policy.source_sha256.len(), 36);
    assert_eq!(policy.native_sha256.len(), 6);
    for hash in policy
        .source_sha256
        .values()
        .chain(policy.native_sha256.values())
    {
        assert_eq!(hash.len(), 64);
        assert!(
            hash.bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
    }
    assert_eq!(
        policy
            .declared
            .iter()
            .filter(|entry| entry.replaced)
            .count(),
        7
    );
    assert_eq!(
        policy
            .declared
            .iter()
            .filter(|entry| entry.config_extended)
            .count(),
        2
    );
    assert_eq!(
        policy
            .declared
            .iter()
            .filter(|entry| entry.public_revoked)
            .count(),
        7
    );
    assert_eq!(
        policy
            .declared
            .iter()
            .filter(|entry| entry.hardened)
            .count(),
        7
    );
    policy
}

fn declarations(policy: &Policy) -> BTreeMap<String, &Declared> {
    let mut declared = BTreeMap::new();
    for entry in &policy.declared {
        let key = metadata::signature(&entry.name, &entry.input_types);
        assert!(declared.insert(key, entry).is_none());
    }
    declared
}

fn assert_old(key: &str, old: &Value, current: &Value, declaration: Option<&Declared>) {
    let mut expected = old.clone();
    // This target comparison keeps every raw OID, owner and identity intact.
    if let Some(entry) = declaration {
        assert!(entry.replaced);
        expected["raw"]["prosrc"] = current["raw"]["prosrc"].clone();
        expected["definition"] = current["definition"].clone();
        if entry.config_extended {
            assert_eq!(
                old["raw"]["proconfig"],
                json!(["search_path=pg_catalog, public"])
            );
            assert_eq!(
                current["raw"]["proconfig"],
                json!(["search_path=pg_catalog, public, pg_temp"])
            );
            expected["raw"]["proconfig"] = current["raw"]["proconfig"].clone();
        }
    }
    if key == "public.assert_instance_accepts_work(pg_catalog.uuid)" {
        assert_eq!(old["raw"]["proacl"], json!(["postgres=X/postgres"]));
        let granted = json!(["postgres=X/postgres", "hephaestus_worker=X/postgres"]);
        assert_eq!(current["raw"]["proacl"], granted);
        expected["raw"]["proacl"] = granted;
    }
    assert_eq!(
        &expected, current,
        "unapproved change to original routine {key}"
    );
}

fn assert_source(routine: &Value, entry: &Declared) {
    let body = metadata::text(&routine["raw"]["prosrc"]);
    assert_eq!(
        format!("{:x}", Sha256::digest(body.as_bytes())),
        entry.body_sha256
    );
    assert_queued_cancel140(routine, entry, false);
    preclosure142::assert_source(routine, entry);
    terminal143::assert_source(routine, entry);
    if entry.public_revoked {
        assert_eq!(routine["raw"]["proacl"], json!(["postgres=X/postgres"]));
    }
    if entry.name == "assert_retained_owned_reuse_context" {
        assert!(
            !entry.replaced && !entry.config_extended && !entry.public_revoked && entry.hardened
        );
        assert_eq!(
            entry.input_types,
            [
                "pg_catalog.uuid",
                "pg_catalog.uuid",
                "pg_catalog.int8",
                "pg_catalog.bytea",
                "pg_catalog.text",
                "pg_catalog.text",
                "pg_catalog.uuid"
            ]
        );
        assert_eq!(
            routine["raw"]["proacl"],
            json!(["postgres=X/postgres", "hephaestus_worker=X/postgres"])
        );
        assert_eq!(routine["raw"]["prosecdef"], true);
        assert_eq!(routine["raw"]["prokind"], "f");
        assert_eq!(routine["raw"]["prorettype"], "2278");
    }
    if entry.hardened || entry.config_extended {
        assert_eq!(
            routine["raw"]["proconfig"],
            json!(["search_path=pg_catalog, public, pg_temp"])
        );
    }
}

fn assert_reference(target: &Snapshot, reference: &Snapshot, routine: &Value, fresh: &Value) {
    let target_language = inventory::language(target, routine);
    let fresh_language = inventory::language(reference, fresh);
    assert_eq!(
        metadata::normalized(routine, &target_language),
        metadata::normalized(fresh, &fresh_language),
        "complete routine metadata must equal independently rendered fresh143"
    );
}

pub(super) fn assert_forward142(before: &Snapshot, after: &Snapshot) -> BTreeSet<String> {
    preclosure142::forward(before, after)
}

pub(super) fn assert_forward140(before: &Snapshot, after: &Snapshot) -> BTreeSet<String> {
    queued_cancel::assert_forward140(before, after)
}

pub(super) fn assert_forward143(before: &Snapshot, after: &Snapshot) -> BTreeSet<String> {
    terminal143::forward(before, after)
}

pub(super) fn replaces143(name: &str) -> bool {
    terminal143::replaces(name)
}
