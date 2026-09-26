use crate::{context::DevContext, process::Result};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf};

mod db_architecture;
mod db_rls;
mod event_architecture;
mod file_length_architecture;
mod layer_architecture;
mod rpc_architecture;
mod rust_architecture;
mod topology_architecture;

const DOCUMENT: &str = "ARCHITECTURE.md";
const CONFIGURATION: &str = "architecture.toml";

const HARNESS_RULE_IDS: [&str; 3] = [
    "ARCH-CARGO-METADATA",
    "ARCH-EXCEPTION-FORMAT",
    "ARCH-RULE-REGISTRY",
];

const REQUIRED_RULE_IDS: [&str; 64] = [
    "ARCH-CONTROLLED-PUBLIC-MODULES",
    "ARCH-CORE-NO-STD-DEPENDENCIES",
    "ARCH-CRATE-LAYERS",
    "ARCH-ENV-ONLY-IN-CONFIG",
    "ARCH-FILESYSTEM-ONLY-IN-ADAPTERS",
    "ARCH-HTTP-ONLY-IN-INTEGRATIONS",
    "ARCH-MAX-FILE-LENGTH",
    "ARCH-PROCESS-ONLY-IN-ADAPTERS",
    "ARCH-VM-PROVIDER-ONLY-IN-COMPOSITION",
    "DB-MIGRATIONS-ONLY-IN-MIGRATIONS",
    "DB-PAGINATION-STABLE-ORDER",
    "DB-RLS-CONTEXT-REQUIRED",
    "DB-SQLX-ONLY-IN-POSTGRES-ADAPTERS",
    "DB-STATIC-SQL",
    "EVT-CANONICAL-ENVELOPE",
    "EVT-CONSUMER-USES-INBOX",
    "EVT-NATS-ONLY-IN-EVENT-ADAPTERS",
    "EVT-OUTBOX-PUBLISHER-ONLY",
    "EVT-REDUCER-COVERAGE",
    "EVT-SIDE-EFFECT-AFTER-DURABLE-CLAIM",
    "EVT-STATE-AND-EVENT-COMMIT-ATOMICALLY",
    "EVT-STREAM-REAUTHORIZATION",
    "EVT-TYPED-ONEOF-PAYLOAD",
    "RPC-AUTHORIZATION-POLICY-DECLARED",
    "RPC-CONNECT-ONLY-IN-TRANSPORT",
    "RPC-DEADLINE-CANCELLATION-PROPAGATION",
    "RPC-ERRORS-MAPPED-AT-BOUNDARY",
    "RPC-GENERATED-FILES-CLEAN",
    "RPC-GENERATED-TYPES-DO-NOT-LEAK-INWARD",
    "RPC-HANDLER-IS-THIN",
    "RPC-LIST-HAS-PAGINATION",
    "RPC-METHOD-IN-SEPARATE-FILE",
    "RPC-MUTATION-HAS-IDEMPOTENCY-KEY",
    "RPC-NON_RPC-HTTP-ALLOWLIST",
    "RPC-NO-ACTOR-IN-REQUEST",
    "RPC-NO-DIRECT-CONNECT-ERROR",
    "RPC-NO-UNTYPED-APPLICATION-PAYLOADS",
    "RPC-QUERY-IDEMPOTENCY-ANNOTATED",
    "RPC-REMOVED-FIELDS-RESERVED",
    "RPC-WATCH-HAS-RESUME-CURSOR",
    "SEC-NO-SENSITIVE-LOG-ARGUMENTS",
    "SEC-NO-SENSITIVE-OUTPUT-FIELDS",
    "SEC-SENTINEL-NO-PLAINTEXT",
    "SEC-SENSITIVE-NO-UNRESTRICTED-FORMAT",
    "SEC-SENSITIVE-REQUEST-ANNOTATED",
    "UI-DECLARED-INTERACTIONS-ONLY",
    "UI-DESIGN-TOKENS-ONLY",
    "UI-LIVE-RENDERS-ONE-PAGE",
    "UI-NO-CLASS-ESCAPE-HATCH",
    "UI-NO-DOM-INJECTION",
    "UI-NO-EXTERNAL-UI-IMPORTS",
    "UI-PAGE-COMPANIONS",
    "UI-PAGE-IS-PURE",
    "UI-PAGE-STATE-COVERAGE",
    "UI-PUBLIC-FACADE-COMPLETE",
    "UI-RAW-HTML-ONLY-IN-COMPONENTS",
    "UI-SHOWCASE-AND-TEST-PARITY",
    "UI-STATE-HAS-NO-HEEX",
    "UI-TIER-DIRECTION",
    "WEB-NO-FILESYSTEM-OR-PROCESS",
    "WEB-NO-HANDWRITTEN-BACKEND-CLIENT",
    "WEB-NO-INFRASTRUCTURE-DEPENDENCIES",
    "WEB-NO-RAW-BACKEND-ERROR",
    "WEB-RPC-CLIENTS-ONLY-IN-STATE",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchitectureConfiguration {
    version: u32,
    enabled_rules: Vec<String>,
    maximum_file_lines: BTreeMap<String, usize>,
    #[serde(default)]
    exceptions: Vec<ArchitectureException>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchitectureException {
    rule_id: String,
    scope: String,
    rationale: String,
    owner: String,
    expires: Option<String>,
    tracking_task: Option<String>,
}

enum ExceptionSelector<'a> {
    Item(&'a str),
    Line(usize),
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
    workspace_members: Vec<String>,
    workspace_root: PathBuf,
}

#[derive(Debug, Deserialize)]
struct CargoPackage {
    id: String,
    name: String,
    manifest_path: PathBuf,
    #[serde(default)]
    metadata: Value,
    #[serde(default)]
    dependencies: Vec<CargoDependency>,
}

#[derive(Debug, Deserialize)]
struct CargoDependency {
    name: String,
    path: Option<PathBuf>,
    #[serde(default)]
    kind: Option<String>,
}

#[derive(Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Diagnostic {
    rule_id: &'static str,
    message: String,
}

impl Diagnostic {
    fn new(rule_id: &'static str, message: impl Into<String>) -> Self {
        Self {
            rule_id,
            message: message.into(),
        }
    }

    fn render(&self) -> String {
        format!(
            "[{}] {} (see {}#architecture-rule-index)",
            self.rule_id, self.message, DOCUMENT
        )
    }
}

#[path = "architecture/architecture_exceptions.rs"]
mod architecture_exceptions;
#[path = "architecture/architecture_io.rs"]
mod architecture_io;
#[path = "architecture/architecture_metadata.rs"]
mod architecture_metadata;
#[path = "architecture/architecture_registry.rs"]
mod architecture_registry;
#[path = "architecture/architecture_runner.rs"]
mod architecture_runner;
#[path = "architecture/architecture_validation.rs"]
mod architecture_validation;

#[cfg(test)]
#[path = "architecture/architecture_tests.rs"]
mod tests;

pub fn run(context: &DevContext) -> Result<()> {
    architecture_runner::run(context)
}

fn render_diagnostics(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(Diagnostic::render)
        .collect::<Vec<_>>()
        .join("\n")
}
