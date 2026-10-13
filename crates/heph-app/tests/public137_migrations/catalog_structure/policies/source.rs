//! Finite policy identities and headers from the existing sealed migration chain.
#[path = "sql.rs"]
mod sql;

use super::super::super::fixture;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Eq, PartialEq)]
pub(super) struct Declaration {
    pub(super) command: &'static str,
    pub(super) permissive: bool,
    pub(super) roles: BTreeSet<String>,
}

pub(super) fn additions() -> BTreeMap<String, Declaration> {
    let policy: Value =
        serde_json::from_str(include_str!("../../catalog_table_columns/policy.json")).unwrap();
    let mut result = BTreeMap::new();
    let mut count = 0;
    for migration in fixture::MIGRATIONS
        .iter()
        .filter(|entry| (108..=138).contains(&entry.version))
    {
        count += 1;
        let name = format!(
            "{:04}_{}.sql",
            migration.version,
            migration.description.replace(' ', "_")
        );
        assert_eq!(
            policy["source_sha256"][&name],
            format!("{:x}", Sha256::digest(migration.sql.as_bytes()))
        );
        for (key, declaration) in declarations(&migration.sql) {
            assert!(
                result.insert(key, declaration).is_none(),
                "duplicate source policy"
            );
        }
    }
    assert_eq!(count, 31);
    assert_eq!(result.len(), 63);
    result
}

pub(super) fn declarations(text: &str) -> BTreeMap<String, Declaration> {
    let mut result = BTreeMap::new();
    let tokens = sql::tokens(text);
    for statement in tokens.split(|token| token == ";") {
        assert!(
            !matches!(
                statement.first().map(String::as_str),
                Some("alter" | "drop")
            ) || statement.get(1).map(String::as_str) != Some("policy"),
            "no policy replacement source"
        );
        if statement.first().map(String::as_str) != Some("create")
            || statement.get(1).map(String::as_str) != Some("policy")
        {
            continue;
        }
        let name = identifier(&statement[2]);
        assert_eq!(statement[3], "on");
        let mut at = 4;
        let first = identifier(&statement[at]);
        at += 1;
        let table = if statement.get(at).map(String::as_str) == Some(".") {
            assert_eq!(first, "public", "declared policy schema");
            at += 2;
            identifier(&statement[at - 1])
        } else {
            first
        };
        let mut declaration = Declaration {
            command: "*",
            permissive: true,
            roles: BTreeSet::from(["PUBLIC".into()]),
        };
        if statement.get(at).map(String::as_str) == Some("as") {
            declaration.permissive = match statement[at + 1].as_str() {
                "permissive" => true,
                "restrictive" => false,
                _ => panic!("policy mode"),
            };
            at += 2;
        }
        if statement.get(at).map(String::as_str) == Some("for") {
            declaration.command = match statement[at + 1].as_str() {
                "all" => "*",
                "select" => "r",
                "insert" => "a",
                "update" => "w",
                "delete" => "d",
                _ => panic!("policy command"),
            };
            at += 2;
        }
        if statement.get(at).map(String::as_str) == Some("to") {
            declaration.roles.clear();
            loop {
                at += 1;
                let role = identifier(&statement[at]);
                assert!(declaration.roles.insert(if role == "public" {
                    "PUBLIC".into()
                } else {
                    role.into()
                }));
                at += 1;
                if statement.get(at).map(String::as_str) != Some(",") {
                    break;
                }
            }
        }
        assert!(
            matches!(
                statement.get(at).map(String::as_str),
                None | Some("using" | "with")
            ),
            "known policy header"
        );
        assert!(
            result
                .insert(format!("public.{table}.{name}"), declaration)
                .is_none(),
            "duplicate declaration"
        );
    }
    result
}

pub(super) fn identifier(value: &str) -> &str {
    assert!(
        value
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_lowercase() || *byte == b'_')
    );
    assert!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    );
    value
}
