//! Inherited rows/new-table emptiness and replay only. The five named upgrade
//! deltas, new-column values and full catalog semantics need separate assertions.
use super::snapshots::{Column, Phase, Row, Snapshot};
use serde::Deserialize;
use std::collections::BTreeSet;

pub const DEFERRED_UPGRADE_TABLES: [&str; 5] = [
    "_sqlx_migrations",
    "application_events",
    "product_event_outbox",
    "application_event_scopes",
    "application_aggregate_versions",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Names {
    #[serde(rename = "source_contract_SHA256")]
    source_contract_sha256: String,
    public107_tables: Vec<String>,
    added_public137_tables: Vec<String>,
    views: Vec<String>,
}

pub fn assert_upgrade(before: &Snapshot, after: &Snapshot) {
    assert_eq!(
        before.phase(),
        Phase::Public107,
        "needs genuine Public107 baseline"
    );
    assert!(
        matches!(
            after.phase(),
            Phase::Public137
                | Phase::Public138
                | Phase::Public139
                | Phase::Public140
                | Phase::Public141
                | Phase::Public142
                | Phase::Public143
        ),
        "upgrade must finish at Public137..Public143"
    );
    let names = declared();
    assert_shape(before, &names);
    assert_shape(after, &names);
    assert!(
        before.views() == after.views(),
        "original view columns changed"
    );
    for name in &names.public107_tables {
        let old = &before.tables()[name];
        let upgraded = &after.tables()[name];
        assert!(
            old.original_rows.is_none(),
            "baseline is already projected: {name}"
        );
        assert_original_columns(&old.columns, &upgraded.columns, name);
        let original_rows = upgraded
            .original_rows
            .as_ref()
            .unwrap_or_else(|| panic!("missing original-column projection: {name}"));
        if !DEFERRED_UPGRADE_TABLES.contains(&name.as_str()) {
            assert_rows(&old.rows, original_rows, name);
        }
    }
    if after.phase() == Phase::Public143 {
        let terminal = &after.tables()["recipe_prepared_instance_terminal_operations143"];
        assert!(
            terminal.rows.is_empty() && terminal.original_rows.is_none(),
            "new143 terminal table must be empty and unprojected"
        );
    }
    for name in &names.added_public137_tables {
        let added = &after.tables()[name];
        assert!(
            added.rows.is_empty(),
            "new table has rows before probes: {name}"
        );
        assert!(
            added.original_rows.is_none(),
            "new table has an old projection: {name}"
        );
    }
}

pub fn assert_replay(before: &Snapshot, after: &Snapshot) {
    assert!(
        matches!(
            before.phase(),
            Phase::Public137
                | Phase::Public138
                | Phase::Public139
                | Phase::Public140
                | Phase::Public141
                | Phase::Public142
                | Phase::Public143
        ),
        "replay baseline must be Public137..Public143"
    );
    assert_eq!(
        before.phase(),
        after.phase(),
        "replay changed migration phase"
    );
    let names = declared();
    assert_shape(before, &names);
    assert_shape(after, &names);
    assert!(
        before.views() == after.views(),
        "replay changed view columns"
    );
    for (name, original) in before.tables() {
        let replayed = &after.tables()[name];
        assert!(
            original.columns == replayed.columns,
            "replay changed columns: {name}"
        );
        // This includes the complete SQLx journal: no table, column or row is
        // deferred during replay, and a journal append changes this multiset.
        assert_rows(&original.rows, &replayed.rows, name);
        if let Some(projected) = &original.original_rows {
            let repeated = replayed
                .original_rows
                .as_ref()
                .unwrap_or_else(|| panic!("replay lost original projection: {name}"));
            assert_rows(projected, repeated, name);
        }
    }
}

fn declared() -> Names {
    let names: Names = serde_json::from_str(include_str!("snapshots/table_names.json"))
        .expect("accepted finite snapshot names");
    assert_eq!(
        names.source_contract_sha256,
        "e87e2d422d17d5ccfc6a2ff4adf0bd186fc6fbdac69eb04433a47193adb939a7",
        "finite names provenance changed"
    );
    let old: BTreeSet<_> = names.public107_tables.iter().map(String::as_str).collect();
    let added: BTreeSet<_> = names
        .added_public137_tables
        .iter()
        .map(String::as_str)
        .collect();
    let views: BTreeSet<_> = names.views.iter().map(String::as_str).collect();
    assert_eq!(names.public107_tables.len(), 153);
    assert_eq!(old.len(), 153, "duplicate baseline table name");
    assert_eq!(names.added_public137_tables.len(), 30);
    assert_eq!(added.len(), 30, "duplicate added table name");
    assert_eq!(names.views.len(), 10);
    assert_eq!(views.len(), 10, "duplicate view name");
    assert!(old.is_disjoint(&added), "added table was already inherited");
    assert!(
        DEFERRED_UPGRADE_TABLES
            .iter()
            .all(|name| old.contains(name)),
        "a deferred family is absent from the inherited names"
    );
    names
}

fn assert_shape(snapshot: &Snapshot, names: &Names) {
    let mut expected: BTreeSet<_> = names.public107_tables.iter().cloned().collect();
    if snapshot.phase() != Phase::Public107 {
        expected.extend(names.added_public137_tables.iter().cloned());
    }
    if snapshot.phase() == Phase::Public143 {
        expected.insert("recipe_prepared_instance_terminal_operations143".to_owned());
    }
    let actual: BTreeSet<_> = snapshot.tables().keys().cloned().collect();
    assert!(
        actual == expected,
        "snapshot table names differ from complete finite set"
    );
    let expected_views: BTreeSet<_> = names.views.iter().cloned().collect();
    let actual_views: BTreeSet<_> = snapshot.views().keys().cloned().collect();
    assert!(
        actual_views == expected_views,
        "snapshot view names differ from finite set"
    );
    for name in &names.public107_tables {
        let projection = &snapshot.tables()[name].original_rows;
        assert!(
            projection.is_some() == (snapshot.phase() != Phase::Public107),
            "phase requires correct original projection presence: {name}"
        );
    }
    if snapshot.phase() != Phase::Public107 {
        for name in &names.added_public137_tables {
            assert!(
                snapshot.tables()[name].original_rows.is_none(),
                "new table has an inherited projection: {name}"
            );
        }
    }
    let journal = &snapshot.tables()["_sqlx_migrations"];
    assert!(
        journal
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .eq([
                "version",
                "description",
                "installed_on",
                "success",
                "checksum",
                "execution_time",
            ]),
        "journal must include all six original columns"
    );
}

fn assert_original_columns(original: &[Column], upgraded: &[Column], name: &str) {
    assert!(
        upgraded.get(..original.len()) == Some(original),
        "original attnum/name/type identity changed: {name}"
    );
}

fn assert_rows(original: &[Row], actual: &[Row], name: &str) {
    // The sealed API supplies C-ordered PostgreSQL JSONB text. Iterator equality
    // retains duplicates and exact numeric text; no keys/sets/JSON rounding.
    // Failure reports the finite table name rather than secret-bearing rows.
    assert!(
        original
            .iter()
            .map(|row| &row.text)
            .eq(actual.iter().map(|row| &row.text)),
        "complete row multiset changed: {name}"
    );
}
