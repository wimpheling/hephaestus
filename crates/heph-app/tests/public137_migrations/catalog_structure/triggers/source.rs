//! Restricted top-level declaration extraction; source bytes are independently sealed.
use super::super::super::fixture;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Declaration {
    pub(super) function: String,
    pub(super) constraint: bool,
}

pub(super) fn additions() -> BTreeMap<String, Declaration> {
    let policy: Value =
        serde_json::from_str(include_str!("../../catalog_table_columns/policy.json")).unwrap();
    let mut result = BTreeMap::new();
    let mut count = 0;
    for migration in fixture::MIGRATIONS
        .iter()
        .filter(|entry| (108..=143).contains(&entry.version))
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
                "duplicate source trigger"
            );
        }
    }
    assert_eq!(count, 36);
    assert_eq!(result.len(), 129);
    assert_eq!(result.values().filter(|entry| entry.constraint).count(), 31);
    result
}

pub(super) fn declarations(sql: &str) -> BTreeMap<String, Declaration> {
    let tokens = tokens(sql);
    let mut result = BTreeMap::new();
    for statement in tokens.split(|token| token == ";") {
        if statement.first().map(String::as_str) != Some("create") {
            continue;
        }
        let constraint = statement.get(1).map(String::as_str) == Some("constraint");
        let offset = if constraint { 2 } else { 1 };
        if statement.get(offset).map(String::as_str) != Some("trigger") {
            continue;
        }
        let name = identifier(&statement[offset + 1]);
        let on = statement
            .iter()
            .position(|token| token == "on")
            .expect("trigger ON relation");
        let table = qualified_name(statement, on + 1);
        let execute = statement
            .windows(2)
            .position(|pair| pair[0] == "execute" && pair[1] == "function")
            .expect("trigger EXECUTE FUNCTION");
        let function = qualified_name(statement, execute + 2);
        assert!(
            result
                .insert(
                    format!("public.{table}.{name}"),
                    Declaration {
                        function,
                        constraint
                    }
                )
                .is_none()
        );
    }
    result
}

fn qualified_name(tokens: &[String], position: usize) -> String {
    let first = identifier(&tokens[position]);
    if tokens.get(position + 1).map(String::as_str) == Some(".") {
        assert_eq!(first, "public", "only declared public schema");
        identifier(&tokens[position + 2]).to_owned()
    } else {
        first.to_owned()
    }
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

fn quoted_end(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut at = start + 1;
    loop {
        let byte = *bytes.get(at).expect("closed SQL quote");
        at += 1;
        if byte == quote {
            if bytes.get(at) == Some(&quote) {
                at += 1;
            } else {
                return at;
            }
        } else if byte == b'\\' && quote == b'\'' {
            bytes.get(at).expect("complete SQL escape");
            at += 1;
        }
    }
}

fn comment_end(bytes: &[u8], start: usize) -> usize {
    let mut depth = 1;
    let mut at = start + 2;
    while depth > 0 {
        assert!(at + 1 < bytes.len(), "closed SQL comment");
        if bytes[at..].starts_with(b"/*") {
            depth += 1;
            at += 2;
        } else if bytes[at..].starts_with(b"*/") {
            depth -= 1;
            at += 2;
        } else {
            at += 1;
        }
    }
    at
}

fn tokens(sql: &str) -> Vec<String> {
    let bytes = sql.as_bytes();
    let mut at = 0;
    let mut result = Vec::new();
    while at < bytes.len() {
        if bytes[at].is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if bytes[at..].starts_with(b"--") {
            at += bytes[at..]
                .iter()
                .position(|byte| *byte == b'\n')
                .unwrap_or(bytes.len() - at);
            continue;
        }
        if bytes[at..].starts_with(b"/*") {
            at = comment_end(bytes, at);
            continue;
        }
        if matches!(bytes[at], b'\'' | b'"') {
            at = quoted_end(bytes, at, bytes[at]);
            result.push("<quoted>".into());
            continue;
        }
        if bytes[at] == b'$' {
            let length = bytes[at + 1..]
                .iter()
                .position(|byte| *byte == b'$')
                .expect("dollar delimiter")
                + 2;
            let delimiter = &sql[at..at + length];
            assert!(
                delimiter[1..delimiter.len() - 1]
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            );
            let body = at + length;
            at = body + sql[body..].find(delimiter).expect("closed dollar body") + length;
            result.push("<body>".into());
            continue;
        }
        let start = at;
        if bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_' {
            at += 1;
            while bytes
                .get(at)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            {
                at += 1;
            }
        } else {
            at += sql[at..].chars().next().unwrap().len_utf8();
        }
        result.push(sql[start..at].to_ascii_lowercase());
    }
    result
}
