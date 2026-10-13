//! Embedded source checks are distinct from the caller's actual installed native
//! asset qualification. Reuse existing seals; do not manufacture a native proof.
use super::super::super::fixture;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub(super) fn qualify() {
    let policy: Value =
        serde_json::from_str(include_str!("../../catalog_routines/policy.json")).unwrap();
    let constraints: Value =
        serde_json::from_str(include_str!("../../catalog_constraints/policy.json")).unwrap();
    assert_eq!(
        fixture::MIGRATIONS
            .iter()
            .map(|entry| entry.version)
            .collect::<Vec<_>>(),
        (1..=143).collect::<Vec<_>>()
    );
    for migration in fixture::MIGRATIONS.iter() {
        let name = format!(
            "{:04}_{}.sql",
            migration.version,
            migration.description.replace(' ', "_")
        );
        let section = if migration.version <= 107 {
            "baseline_sha256"
        } else {
            "source_sha256"
        };
        assert_eq!(
            constraints[section][&name],
            format!("{:x}", Sha256::digest(migration.sql.as_bytes()))
        );
    }
    let source = fixture::MIGRATIONS
        .iter()
        .find(|entry| entry.version == 118)
        .unwrap();
    assert_eq!(
        source
            .sql
            .matches("CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;")
            .count(),
        1
    );
    assert_eq!(policy["native"].as_array().unwrap().len(), 36);
    assert_eq!(
        policy["native_sha256"]["pgcrypto.control"],
        "2a6c81eef34ae6cee9ff5b557f3b6285db7fdd7bc76ee57798e82c945c61c939"
    );
    assert_eq!(
        policy["native_sha256"]["pgcrypto--1.3.sql"],
        "84f8eee1d6b03982fac596988b46bf89d5d5d8790d3133783eedad38bc7d8d38"
    );
}
