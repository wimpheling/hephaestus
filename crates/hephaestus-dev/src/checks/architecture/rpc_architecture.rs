//! Migration-gated structural checks for the Rust RPC boundary.

use super::Diagnostic;
use std::{collections::BTreeMap, ffi::OsStr, fs, path::Path};

const RULES: [&str; 7] = [
    "RPC-CONNECT-ONLY-IN-TRANSPORT",
    "RPC-METHOD-IN-SEPARATE-FILE",
    "RPC-NON_RPC-HTTP-ALLOWLIST",
    "RPC-GENERATED-FILES-CLEAN",
    "RPC-ERRORS-MAPPED-AT-BOUNDARY",
    "RPC-NO-DIRECT-CONNECT-ERROR",
    "RPC-HANDLER-IS-THIN",
];

const GENERATED_LEAK_RULE: &str = "RPC-GENERATED-TYPES-DO-NOT-LEAK-INWARD";

pub(super) fn validate(root: &Path, enabled_rules: &[String], diagnostics: &mut Vec<Diagnostic>) {
    let active = RULES
        .into_iter()
        .chain([GENERATED_LEAK_RULE])
        .filter(|rule| enabled_rules.iter().any(|enabled| enabled == rule))
        .collect::<Vec<_>>();
    if active.is_empty() {
        return;
    }
    validate_generated_layout(root, &active, diagnostics);
    visit_rust_sources(root, &root.join("crates"), &active, diagnostics);
}

pub(super) fn audit(root: &Path) -> BTreeMap<&'static str, usize> {
    let active = RULES
        .into_iter()
        .chain([GENERATED_LEAK_RULE])
        .collect::<Vec<_>>();
    let mut diagnostics = Vec::new();
    validate_generated_layout(root, &active, &mut diagnostics);
    visit_rust_sources(root, &root.join("crates"), &active, &mut diagnostics);
    let mut counts = BTreeMap::new();
    for diagnostic in diagnostics {
        *counts.entry(diagnostic.rule_id).or_insert(0) += 1;
    }
    counts
}

fn validate_generated_layout(root: &Path, active: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    if !active.contains(&"RPC-GENERATED-FILES-CLEAN") {
        return;
    }
    for required in [
        "buf.gen.rust.yaml",
        "buf.gen.elixir.yaml",
        "scripts/check-generated.sh",
        "crates/rpc-proto/src/generated/descriptor.binpb",
        "crates/rpc-proto/src/generated/messages/mod.rs",
        "crates/rpc-proto/src/generated/connect/mod.rs",
        "web/lib/hephaestus_web/rpc/generated",
    ] {
        if !root.join(required).exists() {
            diagnostics.push(Diagnostic::new(
                "RPC-GENERATED-FILES-CLEAN",
                format!("required generated-protocol path is missing: {required}"),
            ));
        }
    }
}

fn visit_rust_sources(
    root: &Path,
    directory: &Path,
    active: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.ends_with("rpc-proto/src/generated") || path.ends_with("target") {
                continue;
            }
            visit_rust_sources(root, &path, active, diagnostics);
        } else if path.extension() == Some(OsStr::new("rs")) {
            let Ok(source) = fs::read_to_string(&path) else {
                continue;
            };
            let relative = path.strip_prefix(root).unwrap_or(&path);
            validate_source(relative, &source, active, diagnostics);
        }
    }
}

fn validate_source(path: &Path, source: &str, active: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    let rendered = path.to_string_lossy();
    if rendered.starts_with("crates/rpc-proto/")
        || rendered.starts_with("crates/hephaestus-dev/")
        || rendered.contains("/tests/fixtures/")
    {
        return;
    }
    let in_rpc = path_has_component(path, "rpc") || path_has_component(path, "composition");
    let rpc_support = matches!(
        path.file_stem().and_then(OsStr::to_str),
        Some("auth" | "error" | "request")
    ) || rendered == "crates/hephaestus-app/src/rpc/mod.rs";

    validate_transport_boundaries(path, source, active, in_rpc, diagnostics);
    validate_handler_structure(path, source, active, in_rpc, rpc_support, diagnostics);
}

fn validate_transport_boundaries(
    path: &Path,
    source: &str,
    active: &[&str],
    in_rpc: bool,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if active.contains(&"RPC-CONNECT-ONLY-IN-TRANSPORT")
        && !in_rpc
        && (source.contains("connectrpc::") || source.contains("ConnectRpcService"))
    {
        diagnostics.push(Diagnostic::new(
            "RPC-CONNECT-ONLY-IN-TRANSPORT",
            format!(
                "{} imports or constructs Connect outside rpc/composition",
                path.display()
            ),
        ));
    }
    if active.contains(&GENERATED_LEAK_RULE)
        && !in_rpc
        && !is_event_transport(path)
        && source.contains("rpc_proto::")
    {
        diagnostics.push(Diagnostic::new(
            "RPC-GENERATED-TYPES-DO-NOT-LEAK-INWARD",
            format!(
                "{} uses generated protocol types below the RPC boundary",
                path.display()
            ),
        ));
    }
    if active.contains(&"RPC-ERRORS-MAPPED-AT-BOUNDARY")
        && !in_rpc
        && (source.contains("ConnectError") || source.contains("ServiceResult"))
    {
        diagnostics.push(Diagnostic::new(
            "RPC-ERRORS-MAPPED-AT-BOUNDARY",
            format!(
                "{} exposes a transport error outside the RPC boundary",
                path.display()
            ),
        ));
    }
}

fn validate_handler_structure(
    path: &Path,
    source: &str,
    active: &[&str],
    in_rpc: bool,
    rpc_support: bool,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if active.contains(&"RPC-NO-DIRECT-CONNECT-ERROR")
        && in_rpc
        && path.file_stem() != Some(OsStr::new("error"))
        && connect_error_constructor(source)
    {
        diagnostics.push(Diagnostic::new(
            "RPC-NO-DIRECT-CONNECT-ERROR",
            format!(
                "{} constructs a Connect error outside rpc/error.rs",
                path.display()
            ),
        ));
    }
    if active.contains(&"RPC-HANDLER-IS-THIN") && in_rpc && !rpc_support {
        let runtime_source = source.split("#[cfg(test)]").next().unwrap_or(source);
        for forbidden in [
            "sqlx::query",
            "sqlx::migrate",
            "std::fs::",
            "tokio::fs::",
            "std::process::Command",
            "tokio::process::Command",
        ] {
            if runtime_source.contains(forbidden) {
                diagnostics.push(Diagnostic::new(
                    "RPC-HANDLER-IS-THIN",
                    format!(
                        "{} performs forbidden handler I/O via {forbidden}",
                        path.display()
                    ),
                ));
                break;
            }
        }
    }
    if active.contains(&"RPC-METHOD-IN-SEPARATE-FILE")
        && in_rpc
        && !rpc_support
        && source.contains("Service for ")
        && path.parent().is_some_and(|parent| parent.ends_with("rpc"))
    {
        diagnostics.push(Diagnostic::new(
            "RPC-METHOD-IN-SEPARATE-FILE",
            format!(
                "{} implements a service in one flat file; use rpc/<service>/<method>.rs",
                path.display()
            ),
        ));
    }
    if active.contains(&"RPC-NON_RPC-HTTP-ALLOWLIST") {
        for route in axum_routes(source) {
            if !allowed_non_rpc_route(route) {
                diagnostics.push(Diagnostic::new(
                    "RPC-NON_RPC-HTTP-ALLOWLIST",
                    format!("{} registers non-RPC HTTP route {route}", path.display()),
                ));
            }
        }
    }
}

fn path_has_component(path: &Path, expected: &str) -> bool {
    path.components()
        .any(|component| component.as_os_str() == OsStr::new(expected))
}

fn is_event_transport(path: &Path) -> bool {
    path_has_component(path, "events")
        || matches!(
            path.file_stem().and_then(OsStr::to_str),
            Some("event_adapter" | "outbox")
        )
}

fn connect_error_constructor(source: &str) -> bool {
    [
        "ConnectError::new(",
        "ConnectError::invalid_argument(",
        "ConnectError::unauthenticated(",
        "ConnectError::permission_denied(",
        "ConnectError::not_found(",
        "ConnectError::internal(",
        "connectrpc::ConnectError::",
    ]
    .iter()
    .any(|constructor| source.contains(constructor))
}

fn axum_routes(source: &str) -> Vec<&str> {
    source
        .lines()
        .filter_map(|line| {
            let (_, remainder) = line.split_once(".route(\"")?;
            remainder.split_once('"').map(|(route, _)| route)
        })
        .collect()
}

fn allowed_non_rpc_route(route: &str) -> bool {
    route == "/healthz"
        || route.starts_with("/git/")
        || route.contains(".git/")
        || matches!(
            route,
            "/{repository}/info/refs"
                | "/{repository}/git-upload-pack"
                | "/{repository}/git-receive-pack"
        )
        || route.starts_with("/.well-known/")
}

#[cfg(test)]
mod tests {
    use super::{RULES, validate_source};
    use crate::checks::architecture::Diagnostic;
    use std::path::Path;

    const INVALID: &str =
        include_str!("../../../tests/fixtures/rpc-architecture/invalid/src/rpc/service.rs");
    const INWARD: &str =
        include_str!("../../../tests/fixtures/rpc-architecture/invalid/src/domain/service.rs");
    const VALID: &str =
        include_str!("../../../tests/fixtures/rpc-architecture/valid/src/rpc/error.rs");

    fn active() -> Vec<&'static str> {
        RULES
            .into_iter()
            .chain(["RPC-GENERATED-TYPES-DO-NOT-LEAK-INWARD"])
            .collect()
    }

    #[test]
    fn invalid_rpc_fixture_covers_layout_routes_errors_and_handler_io() {
        let mut diagnostics = Vec::new();
        validate_source(
            Path::new("crates/example/src/rpc/service.rs"),
            INVALID,
            &active(),
            &mut diagnostics,
        );
        for rule in [
            "RPC-METHOD-IN-SEPARATE-FILE",
            "RPC-NON_RPC-HTTP-ALLOWLIST",
            "RPC-NO-DIRECT-CONNECT-ERROR",
            "RPC-HANDLER-IS-THIN",
        ] {
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.rule_id == rule),
                "fixture did not trigger {rule}"
            );
        }
    }

    #[test]
    fn nested_service_mod_is_checked_as_a_handler() {
        let mut diagnostics = Vec::new();
        validate_source(
            Path::new("crates/example/src/rpc/identity/mod.rs"),
            "async fn resolve() { let _ = sqlx::query(\"SELECT 1\"); }",
            &active(),
            &mut diagnostics,
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule_id == "RPC-HANDLER-IS-THIN")
        );
    }

    #[test]
    fn inward_fixture_rejects_connect_generated_types_and_transport_errors() {
        let mut diagnostics = Vec::new();
        validate_source(
            Path::new("crates/example/src/domain/service.rs"),
            INWARD,
            &active(),
            &mut diagnostics,
        );
        for rule in [
            "RPC-CONNECT-ONLY-IN-TRANSPORT",
            "RPC-ERRORS-MAPPED-AT-BOUNDARY",
            "RPC-GENERATED-TYPES-DO-NOT-LEAK-INWARD",
        ] {
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.rule_id == rule),
                "fixture did not trigger {rule}"
            );
        }
    }

    #[test]
    fn central_error_adapter_and_health_route_are_allowed() {
        let mut diagnostics = Vec::<Diagnostic>::new();
        validate_source(
            Path::new("crates/example/src/rpc/error.rs"),
            VALID,
            &active(),
            &mut diagnostics,
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn rpc_module_helpers_cannot_hide_database_io() {
        let mut diagnostics = Vec::new();
        validate_source(
            Path::new("crates/example/src/rpc/mod.rs"),
            "fn helper() { let _query = sqlx::query(\"SELECT 1\"); }",
            &active(),
            &mut diagnostics,
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule_id == "RPC-HANDLER-IS-THIN")
        );
    }

    #[test]
    fn nested_rpc_services_cannot_hide_database_io() {
        let mut diagnostics = Vec::new();
        validate_source(
            Path::new("crates/example/src/rpc/mod.rs"),
            "mod nested { fn load() { let _query = sqlx::query_as::<_, (i64,)>(\"SELECT 1\"); } }",
            &active(),
            &mut diagnostics,
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule_id == "RPC-HANDLER-IS-THIN")
        );
    }

    #[test]
    fn rpc_constructor_type_plumbing_may_carry_a_pool() {
        let mut diagnostics = Vec::new();
        validate_source(
            Path::new("crates/example/src/rpc/service.rs"),
            "use sqlx::PgPool; struct Service(PgPool); impl Service { fn new(pool: PgPool) -> Self { Self(pool) } }",
            &active(),
            &mut diagnostics,
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn event_transport_may_encode_the_generated_product_envelope() {
        let mut diagnostics = Vec::<Diagnostic>::new();
        validate_source(
            Path::new("crates/example/src/event_adapter.rs"),
            "use rpc_proto::messages::hephaestus::event::v1::ProductEvent;",
            &active(),
            &mut diagnostics,
        );
        assert!(diagnostics.is_empty());
    }
}
