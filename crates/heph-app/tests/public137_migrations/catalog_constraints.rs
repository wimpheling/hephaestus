//! Complete constraint inventory and preservation across public107→143.
//! Before calling, qualify the genuine source/journals/native inputs and pass
//! the routine oracle on these same sealed captures. Fresh metadata is admitted
//! through finite source groups before supplying any generated constraint name.
use super::{catalog::Snapshot, fixture};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[path = "catalog_constraints/expression.rs"]
pub(super) mod expression;
#[path = "catalog_constraints/identity.rs"]
mod identity;
#[path = "catalog_constraints/json_text.rs"]
mod json_text;
#[path = "catalog_constraints/metadata.rs"]
mod metadata;

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(deny_unknown_fields)]
struct Group {
    table: String,
    kind: String,
    columns: Vec<String>,
    foreign_table: Option<String>,
    foreign_columns: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rule {
    group: Group,
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    source_sha256: BTreeMap<String, String>,
    baseline_sha256: BTreeMap<String, String>,
    baseline: Vec<Rule>,
    final_constraints: Vec<Rule>,
    additions: Vec<Rule>,
    structural_removed: Vec<Group>,
    removed: BTreeSet<String>,
    rebuilt: BTreeSet<String>,
}

// Read-only admission shared by the structural oracle. Generated index names
// come from independently admitted source groups, never the upgraded database.
pub(super) fn structural_added_indexes(snapshot: &Snapshot) -> BTreeSet<String> {
    let rules: Vec<_> = policy()
        .additions
        .into_iter()
        .filter(|rule| matches!(rule.group.kind.as_str(), "p" | "u"))
        .collect();
    assert_eq!(rules.len(), 96);
    let constraints = metadata::constraints(snapshot);
    let selected: Vec<_> = constraints
        .values()
        .filter(|value| rules.iter().any(|rule| rule.group == value.group))
        .collect();
    admit(selected.iter().copied(), &rules);
    let relations = identity::relations(snapshot);
    let mut result = BTreeSet::new();
    for constraint in selected {
        let relation = relations[identity::oid(&constraint.value["raw"]["conindid"])];
        assert_eq!(relation["namespace"], "public");
        assert_eq!(relation["raw"]["relkind"], "i");
        assert!(
            result.insert(format!(
                "public.{}",
                identity::text(&relation["raw"]["relname"])
            )),
            "distinct supporting index per admitted PK/UNIQUE"
        );
    }
    assert_eq!(result.len(), 96);
    result
}

pub fn assert_constraints(before: &Snapshot, after: &Snapshot, reference: &Snapshot) {
    let policy = policy();
    context(before, after, reference);
    let old = metadata::constraints(before);
    let upgraded = metadata::constraints(after);
    let fresh = metadata::constraints(reference);
    admit(old.values(), &policy.baseline);
    admit(upgraded.values(), &policy.final_constraints);
    admit(fresh.values(), &policy.final_constraints);
    let mut exceptions: BTreeSet<_> = policy.removed.union(&policy.rebuilt).cloned().collect();
    for group in &policy.structural_removed {
        let selected: Vec<_> = old.values().filter(|value| value.group == *group).collect();
        assert_eq!(
            selected.len(),
            1,
            "Source120 unique original attachment CHECK"
        );
        let key = &selected[0].key;
        assert!(
            !fresh.contains_key(key),
            "source-selected original CHECK removed"
        );
        assert!(exceptions.insert(key.clone()));
    }
    assert!(exceptions.iter().all(|key| old.contains_key(key)));
    assert!(policy.removed.iter().all(|key| !fresh.contains_key(key)));
    assert!(policy.rebuilt.iter().all(|key| fresh.contains_key(key)));
    let canonical: Vec<_> = fresh
        .iter()
        .filter(|(key, _)| !old.contains_key(*key) || policy.rebuilt.contains(*key))
        .map(|(_, value)| value)
        .collect();
    // Every canonical record is bound to a finite declaration and multiplicity.
    // A CHECK group compares every member's complete metadata, never a sample.
    admit(canonical.iter().copied(), &policy.additions);
    let expected: BTreeSet<_> = old
        .keys()
        .filter(|key| !exceptions.contains(*key))
        .chain(canonical.iter().map(|value| &value.key))
        .cloned()
        .collect();
    assert_eq!(fresh.keys().cloned().collect::<BTreeSet<_>>(), expected);
    assert_eq!(upgraded.keys().cloned().collect::<BTreeSet<_>>(), expected);
    for (key, original) in &old {
        if !exceptions.contains(key) {
            assert_eq!(
                original.text, upgraded[key].text,
                "changed old constraint {key}"
            );
            assert_eq!(original.value, upgraded[key].value);
        }
    }
    let old_oids: BTreeSet<_> = old
        .values()
        .map(|value| identity::oid(&value.value["raw"]["oid"]))
        .collect();
    for canonical in canonical {
        let actual = &upgraded[&canonical.key];
        assert!(!old_oids.contains(identity::oid(&actual.value["raw"]["oid"])));
        assert_eq!(
            metadata::normalized(after, actual),
            metadata::normalized(reference, canonical),
            "canonical constraint {}",
            canonical.key
        );
    }
}

fn admit<'a>(values: impl Iterator<Item = &'a metadata::Constraint<'a>>, rules: &[Rule]) {
    let values: Vec<_> = values.collect();
    assert_eq!(
        values.len(),
        rules.len(),
        "complete finite source inventory"
    );
    let mut expected = BTreeMap::new();
    for rule in rules {
        *expected.entry(rule.group.clone()).or_insert(0_usize) += 1;
    }
    let mut observed = BTreeMap::new();
    for value in &values {
        *observed.entry(value.group.clone()).or_insert(0_usize) += 1;
    }
    assert_eq!(observed, expected, "source structural group multiplicity");
    for rule in rules {
        if let Some(name) = &rule.name {
            let key = format!("public.{}.{name}", rule.group.table);
            let matches: Vec<_> = values.iter().filter(|value| value.key == key).collect();
            assert_eq!(matches.len(), 1, "source named constraint {key}");
            assert_eq!(matches[0].group, rule.group);
        }
    }
}

fn policy() -> Policy {
    let policy: Policy =
        serde_json::from_str(include_str!("catalog_constraints/policy.json")).unwrap();
    assert_eq!(policy.source_sha256.len(), 36);
    assert_eq!(policy.baseline_sha256.len(), 107);
    assert_eq!(policy.baseline.len(), 1528);
    assert_eq!(policy.final_constraints.len(), 1997);
    assert_eq!(policy.additions.len(), 474);
    assert_eq!(policy.removed.len(), 1);
    assert_eq!(policy.structural_removed.len(), 1);
    assert_eq!(policy.rebuilt.len(), 3);
    let routine: serde_json::Value =
        serde_json::from_str(include_str!("catalog_routines/policy.json")).unwrap();
    assert_eq!(
        serde_json::to_value(&policy.source_sha256).unwrap(),
        routine["source_sha256"]
    );
    let sources: BTreeMap<_, _> = policy
        .baseline_sha256
        .iter()
        .chain(&policy.source_sha256)
        .collect();
    assert_eq!(sources.len(), fixture::MIGRATIONS.iter().len());
    assert_eq!(
        fixture::MIGRATIONS
            .iter()
            .map(|entry| entry.version)
            .collect::<Vec<_>>(),
        (1..=143).collect::<Vec<_>>()
    );
    for (filename, hash) in sources {
        let (version, description) = filename
            .strip_suffix(".sql")
            .unwrap()
            .split_once('_')
            .unwrap();
        let version: i64 = version.parse().unwrap();
        let migration = fixture::MIGRATIONS
            .iter()
            .find(|entry| entry.version == version)
            .unwrap();
        assert_eq!(
            migration.description.as_ref(),
            description.replace('_', " ")
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(migration.sql.as_bytes())),
            *hash
        );
    }
    policy
}

fn context(before: &Snapshot, after: &Snapshot, reference: &Snapshot) {
    assert_eq!(before.context(), after.context());
    let target = after.context().json();
    let independent = reference.context().json();
    assert_ne!(target["database"], independent["database"]);
    let mut expected = target.clone();
    expected["database"] = independent["database"].clone();
    assert_eq!(
        &expected, independent,
        "only reference database name may differ"
    );
    for snapshot in [before, after, reference] {
        let databases = identity::rows(snapshot, "database");
        assert_eq!(databases.len(), 1);
        assert_eq!(databases[0]["owner"], "postgres");
        assert_eq!(
            databases[0]["raw"]["datname"],
            snapshot.context().json()["database"]
        );
    }
}

// The caller has passed assert_constraints on the same sealed captures. Reuse
// its source policy and exact group selector instead of copying retired keys.
pub(super) fn dependency_changed_keys(before: &Snapshot) -> BTreeSet<String> {
    let policy = policy();
    let original = metadata::constraints(before);
    let mut changed: BTreeSet<_> = policy.removed.union(&policy.rebuilt).cloned().collect();
    for group in &policy.structural_removed {
        let matches: Vec<_> = original
            .values()
            .filter(|value| value.group == *group)
            .collect();
        assert_eq!(matches.len(), 1);
        assert!(changed.insert(matches[0].key.clone()));
    }
    assert!(changed.iter().all(|key| original.contains_key(key)));
    changed
}
