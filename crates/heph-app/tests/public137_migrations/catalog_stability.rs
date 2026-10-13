//! Exact catalog stability only. The other seventeen domains still need
//! finite surviving-identity and independent fresh-reference assertions.
//! The caller must independently establish migration phases via the journal.
#[path = "catalog_stability/initial_privileges.rs"]
mod initial_privileges;

use super::catalog::{Captured, Snapshot};
use std::collections::BTreeSet;

pub fn assert_replay(before: &Snapshot, after: &Snapshot) {
    assert_inventory(before);
    assert_inventory(after);
    assert_captured(before.context(), after.context(), "replay context");
    for name in DOMAIN_NAMES {
        assert_captured(&before.domains()[name], &after.domains()[name], name);
    }
    assert_sequences(before, after);
}

pub fn assert_untouched(before: &Snapshot, after: &Snapshot) {
    initial_privileges::assert_normal_preservation(before, after);
    assert_inventory(before);
    assert_inventory(after);
    assert_captured(before.context(), after.context(), "upgrade context");
    for name in UNTOUCHED_DOMAINS {
        assert_captured(&before.domains()[name], &after.domains()[name], name);
    }
    assert_sequences(before, after);
}

fn assert_inventory(snapshot: &Snapshot) {
    let actual: BTreeSet<&str> = snapshot.domains().keys().map(String::as_str).collect();
    let expected: BTreeSet<&str> = DOMAIN_NAMES.into_iter().collect();
    assert!(actual == expected, "missing or extra catalog domain");
    assert!(
        snapshot.context().json().is_object(),
        "missing catalog context"
    );
    for (name, domain) in snapshot.domains() {
        assert!(
            domain.json().is_array(),
            "catalog inventory is not an array: {name}"
        );
    }
}

fn assert_captured(before: &Captured, after: &Captured, name: &str) {
    assert!(
        before.text() == after.text(),
        "PostgreSQL JSONB text changed: {name}"
    );
    assert!(
        before.json() == after.json(),
        "parsed catalog inventory changed: {name}"
    );
}

fn assert_sequences(before: &Snapshot, after: &Snapshot) {
    assert!(
        before.sequences().keys().eq(after.sequences().keys()),
        "missing or extra actual sequence state"
    );
    for (name, original) in before.sequences() {
        assert_captured(original, &after.sequences()[name], name);
    }
}

// Source108–138 preserve these domains under the checked normal source profile.
// Caller-qualified pgcrypto1.3 control/install bytes contain no ACL statements;
// ordinary post-install grants/revokes do not write pg_init_privs. Extension,
// member and comment domains still need separate finite upgrade assertions.
const UNTOUCHED_DOMAINS: [&str; 23] = [
    "aggregates",
    "collations",
    "database",
    "default_acl",
    "enums",
    "foreign_tables",
    "inheritance",
    "initial_privileges",
    "languages",
    "memberships",
    "namespaces",
    "operators",
    "partitioning",
    "ranges",
    "role_settings",
    "roles",
    "rules",
    "security_labels",
    "sequence_names",
    "sequences",
    "shared_descriptions",
    "shared_security_labels",
    "views",
];

const DOMAIN_NAMES: [&str; 39] = [
    "aggregates",
    "collations",
    "columns",
    "constraints",
    "database",
    "default_acl",
    "defaults",
    "dependencies",
    "descriptions",
    "enums",
    "extension_members",
    "extensions",
    "foreign_tables",
    "indexes",
    "inheritance",
    "initial_privileges",
    "languages",
    "memberships",
    "namespaces",
    "operator_classes",
    "operator_families",
    "operators",
    "partitioning",
    "policies",
    "ranges",
    "relations",
    "role_settings",
    "roles",
    "routines",
    "rules",
    "security_labels",
    "sequence_names",
    "sequences",
    "shared_dependencies",
    "shared_descriptions",
    "shared_security_labels",
    "triggers",
    "types",
    "views",
];
