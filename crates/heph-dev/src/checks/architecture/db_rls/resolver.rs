//! Exact migration-backed SECURITY DEFINER resolver recognition.

use std::{fs, path::Path};

#[derive(Clone, Copy)]
struct ResolverSpec {
    name: &'static str,
    migration: &'static str,
}

const RESOLVER_SPECS: [ResolverSpec; 6] = [
    ResolverSpec {
        name: "authenticate_human_browser_session",
        migration: "migrations/0085_human_browser_sessions.sql",
    },
    ResolverSpec {
        name: "authenticate_ui_browser_session",
        migration: "migrations/0090_ui_browser_session_authentication.sql",
    },
    ResolverSpec {
        name: "resolve_active_ui_generation_host",
        migration: "migrations/0092_ui_browser_host_and_resource_reads.sql",
    },
    ResolverSpec {
        name: "resolve_ui_browser_resource",
        migration: "migrations/0092_ui_browser_host_and_resource_reads.sql",
    },
    ResolverSpec {
        name: "resolve_ui_browser_repository_target_context",
        migration: "migrations/0098_ui_browser_target_context.sql",
    },
    ResolverSpec {
        name: "resolve_ui_browser_repository_git_context",
        migration: "migrations/0097_ui_repository_git_audit_context.sql",
    },
];
pub(super) fn is_allowlisted_resolver(sql: &str, repository_root: &Path) -> bool {
    let sql = strip_sql_comments(sql);
    let normalized = collapse_sql_whitespace(&sql).to_ascii_lowercase();
    if !normalized.starts_with("select ") || normalized.contains(';') {
        return false;
    }
    if [
        " with ",
        " join ",
        " where ",
        " union ",
        " group ",
        " order ",
        " limit ",
        " offset ",
        " having ",
        " into ",
        " returning ",
        " insert ",
        " update ",
        " delete ",
    ]
    .iter()
    .any(|keyword| normalized.contains(keyword))
    {
        return false;
    }
    let Some(from) = token_position(&normalized, "from") else {
        return false;
    };
    let prefix = normalized[..from].trim_end();
    if !prefix.starts_with("select ") || prefix[7..].contains("select") {
        return false;
    }
    let suffix = normalized[from + 4..].trim_start();
    let Some(spec) = RESOLVER_SPECS.iter().find(|spec| {
        let bare = format!("{}(", spec.name);
        let qualified = format!("public.{}(", spec.name);
        suffix.starts_with(&bare) || suffix.starts_with(&qualified)
    }) else {
        return false;
    };
    let function_start = if suffix.starts_with("public.") {
        spec.name.len() + 7
    } else {
        spec.name.len()
    };
    let Some(open) = suffix[function_start..].find('(') else {
        return false;
    };
    let open = function_start + open;
    let Some(close) = suffix.rfind(')') else {
        return false;
    };
    if close < open || !suffix[close + 1..].trim().is_empty() {
        return false;
    }
    if suffix[open + 1..close].contains("select") {
        return false;
    }
    migration_is_security_definer(repository_root, *spec)
}

fn token_position(sql: &str, token: &str) -> Option<usize> {
    let mut offset = 0;
    while let Some(index) = sql[offset..].find(token) {
        let start = offset + index;
        let end = start + token.len();
        let before_ok = start == 0 || !sql.as_bytes()[start - 1].is_ascii_alphanumeric();
        let after_ok = end == sql.len() || !sql.as_bytes()[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return Some(start);
        }
        offset = end;
    }
    None
}

fn collapse_sql_whitespace(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_sql_comments(sql: &str) -> String {
    let mut output = String::with_capacity(sql.len());
    let bytes = sql.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"--") {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            output.push(' ');
        } else if bytes[index..].starts_with(b"/*") {
            index += 2;
            while index + 1 < bytes.len() && !bytes[index..].starts_with(b"*/") {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            output.push(' ');
        } else {
            output.push(bytes[index] as char);
            index += 1;
        }
    }
    output
}

fn migration_is_security_definer(root: &Path, resolver: ResolverSpec) -> bool {
    let Ok(source) = fs::read_to_string(root.join(resolver.migration)) else {
        return false;
    };
    let source = source.to_ascii_lowercase();
    let Some(start) = source.find(&format!("create function {}", resolver.name)) else {
        return false;
    };
    let end = source[start + 1..]
        .find("create function ")
        .map_or(source.len(), |offset| start + 1 + offset);
    source[start..end].contains("security definer")
}
