//! Ordinary public table user attributes only. The caller separately qualifies
//! genuine107/upgraded143/fresh143 journals and complete source/native inputs.
//! Defaults, constraints, relation flags and other attribute kinds remain pending.
use super::catalog::Snapshot;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[path = "catalog_table_columns/json_text.rs"]
pub(super) mod json_text;
#[path = "catalog_table_columns/metadata.rs"]
mod metadata;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    source_sha256: BTreeMap<String, String>,
    original_tables: Vec<String>,
    new_tables: BTreeMap<String, Vec<String>>,
    added_columns: BTreeMap<String, Vec<String>>,
}

pub(super) struct StructuralShapes {
    pub(super) new_tables: BTreeSet<String>,
    pub(super) added_columns: BTreeMap<String, usize>,
}

pub(super) fn structural_shapes() -> StructuralShapes {
    let policy = policy();
    StructuralShapes {
        new_tables: policy.new_tables.into_keys().collect(),
        added_columns: policy
            .added_columns
            .into_iter()
            .map(|(table, columns)| (table, columns.len()))
            .collect(),
    }
}

pub fn assert_table_columns(before: &Snapshot, after: &Snapshot, reference: &Snapshot) {
    context(before, after, reference);
    let policy = policy();
    let old = metadata::tables(before);
    let upgraded = metadata::tables(after);
    let fresh = metadata::tables(reference);
    let original: BTreeSet<_> = policy.original_tables.iter().cloned().collect();
    assert_eq!(old.keys().cloned().collect::<BTreeSet<_>>(), original);
    let expected: BTreeSet<_> = original
        .iter()
        .cloned()
        .chain(policy.new_tables.keys().cloned())
        .collect();
    assert_eq!(expected.len(), 184);
    assert_eq!(upgraded.keys().cloned().collect::<BTreeSet<_>>(), expected);
    assert_eq!(fresh.keys().cloned().collect::<BTreeSet<_>>(), expected);
    for (name, original_table) in &old {
        let current = &upgraded[name];
        let canonical = &fresh[name];
        assert_eq!(
            original_table.oid, current.oid,
            "original table OID must survive"
        );
        let additions = policy
            .added_columns
            .get(name)
            .map_or(&[][..], Vec::as_slice);
        assert_eq!(
            current.columns.len(),
            original_table.columns.len() + additions.len()
        );
        assert_eq!(
            names(current),
            names(canonical),
            "exact full user attribute identities/order"
        );
        for (number, original_column) in &original_table.columns {
            let preserved = current
                .columns
                .get(number)
                .expect("missing original user attribute");
            // Exact PG JSONB bytes retain all old metadata, missing datum numeric
            // precision, helper identities, flags and raw OIDs, including drops.
            assert_eq!(
                original_column.text(),
                preserved.text(),
                "changed original column on {name}"
            );
            assert_eq!(original_column.value(), preserved.value());
        }
        let first = i64::try_from(original_table.columns.len()).unwrap() + 1;
        for (offset, added_name) in additions.iter().enumerate() {
            let number = first + i64::try_from(offset).unwrap();
            let added = &current.columns[&number];
            assert_eq!(added.value()["raw"]["attname"], *added_name);
            assert_eq!(added.value()["raw"]["attisdropped"], false);
            assert_eq!(
                metadata::normalized(added),
                metadata::normalized(&canonical.columns[&number])
            );
        }
    }
    for (name, declared_columns) in &policy.new_tables {
        let current = &upgraded[name];
        let canonical = &fresh[name];
        assert_eq!(
            names(current),
            *declared_columns,
            "complete source-declared new column set/order"
        );
        assert_eq!(names(current), names(canonical));
        for (number, column) in &current.columns {
            assert_eq!(column.value()["raw"]["attisdropped"], false);
            assert_eq!(
                metadata::normalized(column),
                metadata::normalized(&canonical.columns[number])
            );
        }
    }
}

fn names(table: &metadata::Table<'_>) -> Vec<String> {
    table
        .columns
        .values()
        .map(|column| metadata::text(&column.value()["raw"]["attname"]).to_owned())
        .collect()
}

fn policy() -> Policy {
    let policy: Policy =
        serde_json::from_str(include_str!("catalog_table_columns/policy.json")).unwrap();
    assert_eq!(policy.original_tables.len(), 153);
    assert_eq!(
        policy.original_tables.iter().collect::<BTreeSet<_>>().len(),
        153
    );
    assert_eq!(policy.new_tables.len(), 31);
    assert_eq!(
        policy.added_columns.values().map(Vec::len).sum::<usize>(),
        27
    );
    assert_eq!(policy.source_sha256.len(), 36);
    for (table, columns) in policy.new_tables.iter().chain(&policy.added_columns) {
        assert_eq!(columns.iter().collect::<BTreeSet<_>>().len(), columns.len());
        assert!(!columns.is_empty());
        if policy.added_columns.contains_key(table) {
            assert!(policy.original_tables.contains(table));
        } else {
            assert!(!policy.original_tables.contains(table));
        }
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
    assert_eq!(
        before.context(),
        after.context(),
        "target database/context exact"
    );
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
