//! This check binds embedded sources, not installed native files. The caller must
//! qualify the exact pgcrypto1.3 control/install assets sealed by routine policy.
use super::super::{catalog::Snapshot, fixture};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub(super) fn assert_normal_preservation(before: &Snapshot, after: &Snapshot) {
    normal_source_profile();
    let original = &before.domains()["initial_privileges"];
    let current = &after.domains()["initial_privileges"];
    assert_records(
        original.json(),
        original.text(),
        current.json(),
        current.text(),
    );
}

fn normal_source_profile() {
    let constraints: Value =
        serde_json::from_str(include_str!("../catalog_constraints/policy.json")).unwrap();
    let routines: Value =
        serde_json::from_str(include_str!("../catalog_routines/policy.json")).unwrap();
    assert_eq!(
        fixture::MIGRATIONS
            .iter()
            .map(|entry| entry.version)
            .collect::<Vec<_>>(),
        (1..=143).collect::<Vec<_>>()
    );
    for migration in fixture::MIGRATIONS.iter() {
        let suffix = format!(
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
            constraints[section][&suffix],
            format!("{:x}", Sha256::digest(migration.sql.as_bytes())),
            "exact normal source profile"
        );
    }
    // Reviewed scripts: only CREATE FUNCTION in pgcrypto1.3, no ACL statements;
    // no ALTER DEFAULT PRIVILEGES / ALTER EXTENSION ADD in the sealed chain.
    assert_eq!(
        routines["native_sha256"]["pgcrypto--1.3.sql"],
        "84f8eee1d6b03982fac596988b46bf89d5d5d8790d3133783eedad38bc7d8d38"
    );
    assert_eq!(
        routines["native_sha256"]["pgcrypto.control"],
        "2a6c81eef34ae6cee9ff5b557f3b6285db7fdd7bc76ee57798e82c945c61c939"
    );
}

fn assert_records(before: &Value, before_text: &str, after: &Value, after_text: &str) {
    assert_eq!(
        before_text, after_text,
        "lossless initial privileges changed"
    );
    assert_eq!(before, after, "initial privileges changed");
}

#[cfg(test)]
mod tests {
    use super::{assert_records, normal_source_profile};
    use serde_json::json;

    #[test]
    fn embedded_normal_migrator_matches_existing_sealed_profile() {
        normal_source_profile();
    }

    #[test]
    fn initial_privilege_acl_identity_and_inventory_substitutions_are_denied() {
        let original = json!([{"raw": {"objoid": "20000", "classoid": "1255", "objsubid": 0,
            "privtype": "e", "initprivs": ["postgres=X/postgres"]}}]);
        let text = original.to_string();
        assert_records(&original, &text, &original, &text);
        for field in ["objoid", "classoid", "objsubid", "privtype", "initprivs"] {
            let mut changed = original.clone();
            changed[0]["raw"][field] = json!("substituted");
            let changed_text = changed.to_string();
            assert!(
                std::panic::catch_unwind(|| assert_records(
                    &original,
                    &text,
                    &changed,
                    &changed_text
                ))
                .is_err()
            );
        }
        for changed in [json!([]), json!([original[0], original[0]])] {
            let changed_text = changed.to_string();
            assert!(
                std::panic::catch_unwind(|| assert_records(
                    &original,
                    &text,
                    &changed,
                    &changed_text
                ))
                .is_err()
            );
        }
        assert!(
            std::panic::catch_unwind(|| assert_records(
                &original,
                &text,
                &original,
                "different raw text"
            ))
            .is_err()
        );
    }
}
