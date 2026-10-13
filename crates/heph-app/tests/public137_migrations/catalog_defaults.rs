//! Default and stored generated expressions. The caller independently qualifies
//! genuine `SQLx` journals, source/native inputs and the fresh reference database.
//! This assertion does not certify other catalog domains or table row contents.
use super::catalog::Snapshot;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[path = "catalog_defaults/json_text.rs"]
mod json_text;
#[path = "catalog_defaults/metadata.rs"]
mod metadata;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rule {
    declaration: String,
    generated: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    source_sha256: BTreeMap<String, String>,
    added_defaults: BTreeMap<String, Rule>,
    old_default_changes: Vec<String>,
}

pub fn assert_defaults(before: &Snapshot, after: &Snapshot, reference: &Snapshot) {
    context(before, after, reference);
    let policy = policy();
    let old = metadata::defaults(before);
    let upgraded = metadata::defaults(after);
    let fresh = metadata::defaults(reference);
    let added: BTreeSet<_> = policy.added_defaults.keys().cloned().collect();
    assert!(old.keys().all(|name| !added.contains(name)));
    let expected: BTreeSet<_> = old.keys().cloned().chain(added).collect();
    assert_eq!(upgraded.keys().cloned().collect::<BTreeSet<_>>(), expected);
    assert_eq!(fresh.keys().cloned().collect::<BTreeSet<_>>(), expected);
    for (identity, original) in &old {
        let preserved = &upgraded[identity];
        // Whole original PostgreSQL JSONB text includes raw OIDs, node tree,
        // attribute number, qualified helpers and exact server deparser output.
        assert_eq!(
            original.text, preserved.text,
            "changed old default {identity}"
        );
        assert_eq!(original.value, preserved.value);
        assert_eq!(original.generated, preserved.generated);
    }
    for (identity, rule) in &policy.added_defaults {
        let actual = &upgraded[identity];
        let canonical = &fresh[identity];
        assert_eq!(actual.generated, rule.generated);
        assert_eq!(canonical.generated, rule.generated);
        // No pg_node_tree edits: all declared new expressions use builtins only.
        assert_eq!(
            metadata::normalized(actual),
            metadata::normalized(canonical)
        );
    }
}

fn policy() -> Policy {
    let policy: Policy =
        serde_json::from_str(include_str!("catalog_defaults/policy.json")).unwrap();
    assert_eq!(policy.source_sha256.len(), 36);
    assert_eq!(policy.added_defaults.len(), 25);
    assert!(policy.old_default_changes.is_empty());
    assert_eq!(
        policy
            .added_defaults
            .values()
            .filter(|rule| rule.generated)
            .count(),
        1
    );
    for (name, rule) in &policy.added_defaults {
        let parts: Vec<_> = name.split('.').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "public");
        for part in parts {
            assert!(!part.is_empty());
            assert!(
                part.bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
            );
        }
        assert!(
            rule.declaration.contains("DEFAULT")
                || rule.declaration.contains("GENERATED ALWAYS AS")
        );
    }
    for hash in policy.source_sha256.values() {
        assert_eq!(hash.len(), 64);
        assert!(
            hash.bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
    }
    policy
}

fn context(before: &Snapshot, after: &Snapshot, reference: &Snapshot) {
    assert_eq!(before.context(), after.context(), "target context exact");
    let target = after.context().json();
    let independent = reference.context().json();
    assert_ne!(target["database"], independent["database"]);
    let mut expected = target.clone();
    expected["database"] = independent["database"].clone();
    assert_eq!(
        &expected, independent,
        "only independent database name may differ"
    );
    for snapshot in [before, after, reference] {
        let databases = metadata::rows(snapshot, "database");
        assert_eq!(databases.len(), 1);
        assert_eq!(databases[0]["owner"], "postgres");
        assert_eq!(
            databases[0]["raw"]["datname"],
            snapshot.context().json()["database"]
        );
    }
}
