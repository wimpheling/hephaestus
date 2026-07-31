use crate::{
    context::DevContext,
    process::{DevError, Result},
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

mod db_architecture;
mod event_architecture;
mod layer_architecture;
mod rpc_architecture;
mod rust_architecture;

const DOCUMENT: &str = "ARCHITECTURE.md";
const CONFIGURATION: &str = "architecture.toml";

const HARNESS_RULE_IDS: [&str; 3] = [
    "ARCH-CARGO-METADATA",
    "ARCH-EXCEPTION-FORMAT",
    "ARCH-RULE-REGISTRY",
];

const REQUIRED_RULE_IDS: [&str; 59] = [
    "ARCH-CONTROLLED-PUBLIC-MODULES",
    "ARCH-CRATE-LAYERS",
    "ARCH-ENV-ONLY-IN-CONFIG",
    "ARCH-FILESYSTEM-ONLY-IN-ADAPTERS",
    "ARCH-HTTP-ONLY-IN-INTEGRATIONS",
    "ARCH-MAX-FILE-LENGTH",
    "ARCH-PROCESS-ONLY-IN-ADAPTERS",
    "ARCH-VM-PROVIDER-ONLY-IN-COMPOSITION",
    "DB-MIGRATIONS-ONLY-IN-MIGRATIONS",
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

pub fn run(context: &DevContext) -> Result<()> {
    let root = &context.repository_root;
    let document = fs::read_to_string(root.join(DOCUMENT)).map_err(|error| {
        DevError::Invalid(
            Diagnostic::new(
                "ARCH-RULE-REGISTRY",
                format!("cannot read {}: {error}", root.join(DOCUMENT).display()),
            )
            .render(),
        )
    })?;
    let configuration = read_configuration(root)?;
    let metadata = cargo_metadata(root)?;

    let diagnostics = validate_repository(root, &document, &configuration, &metadata);
    if diagnostics.is_empty() {
        println!(
            "architecture checks passed ({} enabled; {} migration-gated)",
            configuration.enabled_rules.len(),
            migration_gated_rule_count(&configuration.enabled_rules)
        );
        println!("enabled rules: {}", configuration.enabled_rules.join(", "));
        println!(
            "migration-gated families: ARCH, DB, EVT, RPC, SEC, UI, WEB; enable each rule only after its repository-wide migration"
        );
        let rpc_audit = rpc_architecture::audit(root);
        if rpc_audit.is_empty() {
            println!("migration-gated RPC structural dry-run: clean");
        } else {
            println!(
                "migration-gated RPC structural dry-run: {}",
                rpc_audit
                    .iter()
                    .map(|(rule, count)| format!("{rule}={count}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        let event_audit = event_architecture::audit(root);
        if event_audit.is_empty() {
            println!("migration-gated event structural dry-run: clean");
        } else {
            println!(
                "migration-gated event structural dry-run: {}",
                event_audit
                    .iter()
                    .map(|(rule, count)| format!("{rule}={count}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        let database_audit = db_architecture::audit(root, &metadata);
        if database_audit.is_empty() {
            println!("migration-gated database structural dry-run: clean");
        } else {
            println!(
                "migration-gated database structural dry-run: {}",
                database_audit
                    .iter()
                    .map(|(rule, count)| format!("{rule}={count}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        let rust_audit = rust_architecture::audit(root);
        if rust_audit.is_empty() {
            println!("migration-gated Rust semantic dry-run: clean");
        } else {
            println!(
                "migration-gated Rust semantic dry-run: {}",
                rust_audit
                    .iter()
                    .map(|(rule, count)| format!("{rule}={count}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        Ok(())
    } else {
        Err(DevError::Invalid(render_diagnostics(&diagnostics)))
    }
}

fn migration_gated_rule_count(enabled_rules: &[String]) -> usize {
    let enabled_catalogue_rules = enabled_rules
        .iter()
        .filter(|rule_id| REQUIRED_RULE_IDS.contains(&rule_id.as_str()))
        .count();
    REQUIRED_RULE_IDS.len() - enabled_catalogue_rules
}

fn read_configuration(root: &Path) -> Result<ArchitectureConfiguration> {
    let path = root.join(CONFIGURATION);
    let source = fs::read_to_string(&path).map_err(|error| {
        DevError::Invalid(
            Diagnostic::new(
                "ARCH-EXCEPTION-FORMAT",
                format!("cannot read {}: {error}", path.display()),
            )
            .render(),
        )
    })?;
    toml::from_str(&source).map_err(|error| {
        DevError::Invalid(
            Diagnostic::new(
                "ARCH-EXCEPTION-FORMAT",
                format!("cannot parse {}: {error}", path.display()),
            )
            .render(),
        )
    })
}

fn cargo_metadata(root: &Path) -> Result<CargoMetadata> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(DevError::Invalid(
            Diagnostic::new(
                "ARCH-CARGO-METADATA",
                format!(
                    "cargo metadata failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            )
            .render(),
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        DevError::Invalid(
            Diagnostic::new(
                "ARCH-CARGO-METADATA",
                format!("cargo metadata returned invalid JSON: {error}"),
            )
            .render(),
        )
    })
}

fn validate_repository(
    root: &Path,
    document: &str,
    configuration: &ArchitectureConfiguration,
    metadata: &CargoMetadata,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    validate_rule_registry(document, configuration, &mut diagnostics);
    validate_configuration(configuration, &mut diagnostics);
    validate_exceptions(root, configuration, &mut diagnostics);
    validate_metadata(root, metadata, &mut diagnostics);
    layer_architecture::validate(&configuration.enabled_rules, metadata, &mut diagnostics);
    db_architecture::validate(
        root,
        &configuration.enabled_rules,
        metadata,
        &mut diagnostics,
    );
    event_architecture::validate(root, &configuration.enabled_rules, &mut diagnostics);
    rpc_architecture::validate(root, &configuration.enabled_rules, &mut diagnostics);
    rust_architecture::validate(root, &configuration.enabled_rules, &mut diagnostics);
    diagnostics.sort();
    diagnostics
}

fn validate_rule_registry(
    document: &str,
    configuration: &ArchitectureConfiguration,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let known_rules = known_rule_ids();
    let mut indexed_rules = BTreeSet::new();
    for line in document.lines().filter(|line| line.starts_with("| `")) {
        let cells = line
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect::<Vec<_>>();
        let Some(rule_id) = cells
            .first()
            .and_then(|cell| cell.strip_prefix('`'))
            .and_then(|cell| cell.strip_suffix('`'))
        else {
            continue;
        };
        if !known_rules.contains(rule_id) {
            diagnostics.push(Diagnostic::new(
                "ARCH-RULE-REGISTRY",
                format!("{DOCUMENT} indexes unknown invariant {rule_id}"),
            ));
            continue;
        }
        if !indexed_rules.insert(rule_id) {
            diagnostics.push(Diagnostic::new(
                "ARCH-RULE-REGISTRY",
                format!("{DOCUMENT} indexes {rule_id} more than once"),
            ));
        }
        validate_index_row(rule_id, &cells, configuration, diagnostics);
    }
    for rule_id in known_rules.difference(&indexed_rules) {
        diagnostics.push(Diagnostic::new(
            "ARCH-RULE-REGISTRY",
            format!("{DOCUMENT} does not index required invariant {rule_id}"),
        ));
    }

    let mut seen = BTreeSet::new();
    for rule_id in &configuration.enabled_rules {
        if !known_rule_ids().contains(rule_id.as_str()) {
            diagnostics.push(Diagnostic::new(
                "ARCH-RULE-REGISTRY",
                format!("enabled rule {rule_id} is absent from the stable registry"),
            ));
        }
        if !seen.insert(rule_id) {
            diagnostics.push(Diagnostic::new(
                "ARCH-RULE-REGISTRY",
                format!("enabled rule {rule_id} is listed more than once"),
            ));
        }
    }
}

fn validate_index_row(
    rule_id: &str,
    cells: &[&str],
    configuration: &ArchitectureConfiguration,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if cells.len() != 5 {
        diagnostics.push(Diagnostic::new(
            "ARCH-RULE-REGISTRY",
            format!("{DOCUMENT} row for {rule_id} must have exactly five index columns"),
        ));
        return;
    }
    let Some((class, state)) = cells[1].rsplit_once(" / ") else {
        diagnostics.push(Diagnostic::new(
            "ARCH-RULE-REGISTRY",
            format!("{DOCUMENT} row for {rule_id} lacks `Class / state`"),
        ));
        return;
    };
    if class.trim().is_empty() || !matches!(state, "harness" | "migration-gated") {
        diagnostics.push(Diagnostic::new(
            "ARCH-RULE-REGISTRY",
            format!("{DOCUMENT} row for {rule_id} has an invalid class or state"),
        ));
    }
    let expected_state = if configuration
        .enabled_rules
        .iter()
        .any(|enabled| enabled == rule_id)
    {
        "harness"
    } else {
        "migration-gated"
    };
    if state != expected_state {
        diagnostics.push(Diagnostic::new(
            "ARCH-RULE-REGISTRY",
            format!(
                "{DOCUMENT} row for {rule_id} has state {state}; configuration requires {expected_state}"
            ),
        ));
    }
    if cells[2].trim().is_empty() || cells[4].trim().is_empty() {
        diagnostics.push(Diagnostic::new(
            "ARCH-RULE-REGISTRY",
            format!("{DOCUMENT} row for {rule_id} requires rationale, scope, and remediation"),
        ));
    }
    let command = cells[3].trim_matches('`');
    if !matches!(
        command,
        "cargo dev check architecture"
            | "cargo dev check protobuf"
            | "cargo dev check rust"
            | "cargo dev check phoenix"
            | "cargo dev check ui"
            | "cargo dev check full"
    ) {
        diagnostics.push(Diagnostic::new(
            "ARCH-RULE-REGISTRY",
            format!("{DOCUMENT} row for {rule_id} has an unknown enforcement command"),
        ));
    }
}

fn validate_configuration(
    configuration: &ArchitectureConfiguration,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if configuration.version != 1 {
        diagnostics.push(Diagnostic::new(
            "ARCH-RULE-REGISTRY",
            format!(
                "unsupported architecture configuration version {}; expected 1",
                configuration.version
            ),
        ));
    }
    if configuration.maximum_file_lines.is_empty()
        || configuration
            .maximum_file_lines
            .values()
            .any(|threshold| *threshold == 0)
    {
        diagnostics.push(Diagnostic::new(
            "ARCH-RULE-REGISTRY",
            "maximum file lengths must be configured centrally as positive line counts",
        ));
    }
}

fn validate_exceptions(
    root: &Path,
    configuration: &ArchitectureConfiguration,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut seen = BTreeSet::new();
    for exception in &configuration.exceptions {
        if !known_rule_ids().contains(exception.rule_id.as_str()) {
            diagnostics.push(Diagnostic::new(
                "ARCH-EXCEPTION-FORMAT",
                format!("exception names unknown rule {}", exception.rule_id),
            ));
        }
        if exception.rationale.trim().is_empty() || exception.owner.trim().is_empty() {
            diagnostics.push(Diagnostic::new(
                "ARCH-EXCEPTION-FORMAT",
                format!(
                    "exception for {} requires non-empty rationale and owner",
                    exception.rule_id
                ),
            ));
        }
        let has_expiry = exception
            .expires
            .as_deref()
            .is_some_and(|expiry| !expiry.trim().is_empty());
        let has_tracking_task = exception
            .tracking_task
            .as_deref()
            .is_some_and(|task| !task.trim().is_empty());
        if !has_expiry && !has_tracking_task {
            diagnostics.push(Diagnostic::new(
                "ARCH-EXCEPTION-FORMAT",
                format!(
                    "exception for {} requires an expiry or tracking task",
                    exception.rule_id
                ),
            ));
        }
        if exception
            .expires
            .as_deref()
            .is_some_and(|date| !date_shape(date))
        {
            diagnostics.push(Diagnostic::new(
                "ARCH-EXCEPTION-FORMAT",
                format!(
                    "exception for {} has invalid YYYY-MM-DD expiry",
                    exception.rule_id
                ),
            ));
        }
        if let Some(task) = exception
            .tracking_task
            .as_deref()
            .filter(|task| !task.trim().is_empty())
        {
            if let Err(reason) = validate_tracking_task(root, task) {
                diagnostics.push(Diagnostic::new(
                    "ARCH-EXCEPTION-FORMAT",
                    format!(
                        "exception for {} has unresolved tracking task `{task}`: {reason}",
                        exception.rule_id
                    ),
                ));
            }
        }
        if exception.expires.as_deref().is_some_and(expiry_has_passed) {
            diagnostics.push(Diagnostic::new(
                "ARCH-EXCEPTION-FORMAT",
                format!("exception for {} has expired", exception.rule_id),
            ));
        }
        if !seen.insert((&exception.rule_id, &exception.scope)) {
            diagnostics.push(Diagnostic::new(
                "ARCH-EXCEPTION-FORMAT",
                format!(
                    "exception for {} duplicates exact scope `{}`",
                    exception.rule_id, exception.scope
                ),
            ));
        }
        if let Err(reason) = validate_exact_scope(root, &exception.scope) {
            diagnostics.push(Diagnostic::new(
                "ARCH-EXCEPTION-FORMAT",
                format!(
                    "exception for {} has invalid scope `{}`: {reason}",
                    exception.rule_id, exception.scope
                ),
            ));
        }
    }
}

fn validate_tracking_task(root: &Path, task: &str) -> std::result::Result<(), &'static str> {
    let relative = Path::new(task);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        return Err("use a repository-relative path without parent traversal");
    }
    if !root.join(relative).is_file() {
        return Err("task file does not exist");
    }
    Ok(())
}

fn validate_exact_scope(root: &Path, scope: &str) -> std::result::Result<(), &'static str> {
    if scope.contains(['*', '?', '[', ']']) {
        return Err("globs are forbidden");
    }

    let (path_text, selector) = if let Some((path, item)) = scope.split_once('#') {
        if item.trim().is_empty() {
            return Err("the item selector is empty");
        }
        (path, ExceptionSelector::Item(item))
    } else if let Some((path, line)) = scope.rsplit_once(':') {
        let Ok(line) = line.parse::<usize>() else {
            return Err("the line selector is invalid");
        };
        if line == 0 {
            return Err("the line selector is invalid");
        }
        (path, ExceptionSelector::Line(line))
    } else {
        return Err("use path:line or path#item");
    };

    let relative = Path::new(path_text);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        return Err("scope must be a repository-relative path without parent traversal");
    }
    let target = root.join(relative);
    if target.is_dir() {
        return Err("directory-wide exceptions are forbidden");
    }
    if !target.is_file() {
        return Err("scoped file does not exist");
    }
    let source = fs::read_to_string(target).map_err(|_| "scoped file is not readable text")?;
    match selector {
        ExceptionSelector::Item(item) if !source.contains(item) => {
            return Err("scoped item does not exist in the file");
        }
        ExceptionSelector::Line(line) if source.lines().count() < line => {
            return Err("scoped line is beyond the end of the file");
        }
        ExceptionSelector::Item(_) | ExceptionSelector::Line(_) => {}
    }
    Ok(())
}

fn validate_metadata(root: &Path, metadata: &CargoMetadata, diagnostics: &mut Vec<Diagnostic>) {
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let metadata_root = metadata
        .workspace_root
        .canonicalize()
        .unwrap_or_else(|_| metadata.workspace_root.clone());
    if canonical_root != metadata_root {
        diagnostics.push(Diagnostic::new(
            "ARCH-CARGO-METADATA",
            format!(
                "cargo metadata workspace root {} does not match repository root {}",
                metadata.workspace_root.display(),
                root.display()
            ),
        ));
    }

    let packages_by_id: BTreeMap<_, _> = metadata
        .packages
        .iter()
        .map(|package| (package.id.as_str(), package))
        .collect();
    for member in &metadata.workspace_members {
        let Some(package) = packages_by_id.get(member.as_str()) else {
            diagnostics.push(Diagnostic::new(
                "ARCH-CARGO-METADATA",
                format!("workspace member {member} has no package record"),
            ));
            continue;
        };
        validate_package(root, package, diagnostics);
    }
}

fn validate_package(root: &Path, package: &CargoPackage, diagnostics: &mut Vec<Diagnostic>) {
    let manifest = package
        .manifest_path
        .canonicalize()
        .unwrap_or_else(|_| package.manifest_path.clone());
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if !manifest.starts_with(&canonical_root) || !manifest.is_file() {
        diagnostics.push(Diagnostic::new(
            "ARCH-CARGO-METADATA",
            format!(
                "workspace package {} has manifest outside the repository or missing: {}",
                package.name,
                package.manifest_path.display()
            ),
        ));
    }

    // Read these fields now so the stable metadata model cannot silently drift
    // before layer/dependency rules are activated by later constraints.
    let _architecture_declaration = package.metadata.get("hephaestus");
    for dependency in &package.dependencies {
        let Some(path) = &dependency.path else {
            continue;
        };
        let dependency_path = path.canonicalize().unwrap_or_else(|_| path.clone());
        if !dependency_path.starts_with(&canonical_root) {
            diagnostics.push(Diagnostic::new(
                "ARCH-CARGO-METADATA",
                format!(
                    "workspace package {} has repository-escaping path dependency {} at {}",
                    package.name,
                    dependency.name,
                    path.display()
                ),
            ));
        }
    }
}

fn date_shape(date: &str) -> bool {
    let bytes = date.as_bytes();
    if !(bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit()))
    {
        return false;
    }
    let Ok(year) = date[0..4].parse::<u32>() else {
        return false;
    };
    let Ok(month) = date[5..7].parse::<usize>() else {
        return false;
    };
    let Ok(day) = date[8..10].parse::<u32>() else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_lengths = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    month_lengths
        .get(month.saturating_sub(1))
        .is_some_and(|maximum| month > 0 && day > 0 && day <= *maximum)
}

fn expiry_has_passed(expiry: &str) -> bool {
    date_shape(expiry) && expiry < time::OffsetDateTime::now_utc().date().to_string().as_str()
}

fn known_rule_ids() -> BTreeSet<&'static str> {
    HARNESS_RULE_IDS
        .into_iter()
        .chain(REQUIRED_RULE_IDS)
        .collect()
}

fn render_diagnostics(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(Diagnostic::render)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::{
        ArchitectureConfiguration, CargoDependency, CargoMetadata, CargoPackage, Diagnostic,
        REQUIRED_RULE_IDS, known_rule_ids, migration_gated_rule_count, read_configuration,
        validate_exact_scope, validate_metadata, validate_repository,
    };
    use serde_json::Value;
    use std::path::Path;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/architecture")
            .join(name)
    }

    fn valid_document(configuration: &ArchitectureConfiguration) -> String {
        known_rule_ids()
            .into_iter()
            .map(|rule_id| {
                let state = if configuration
                    .enabled_rules
                    .iter()
                    .any(|enabled| enabled == rule_id)
                {
                    "harness"
                } else {
                    "migration-gated"
                };
                format!(
                    "| `{rule_id}` | Structural / {state} | Fixture rationale and scope. | `cargo dev check architecture` | Fixture remediation. |"
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn migration_gated_count_excludes_harness_only_rules() {
        let enabled_rules = vec![
            "ARCH-RULE-REGISTRY".to_owned(),
            "UI-TIER-DIRECTION".to_owned(),
            "UI-PAGE-STATE-COVERAGE".to_owned(),
        ];

        assert_eq!(
            migration_gated_rule_count(&enabled_rules),
            REQUIRED_RULE_IDS.len() - 2
        );
    }

    #[test]
    fn valid_fixture_has_a_narrow_accountable_exception() {
        let root = fixture("valid");
        let configuration = read_configuration(&root).expect("valid fixture configuration");
        assert_eq!(configuration.exceptions.len(), 1);
        assert!(validate_exact_scope(&root, &configuration.exceptions[0].scope).is_ok());

        let document = valid_document(&configuration);
        let metadata = CargoMetadata {
            packages: vec![CargoPackage {
                id: "fixture 0.1.0".into(),
                name: "fixture".into(),
                manifest_path: root.join("src/lib.rs"),
                metadata: Value::Null,
                dependencies: Vec::new(),
            }],
            workspace_members: vec!["fixture 0.1.0".into()],
            workspace_root: root.clone(),
        };
        assert!(validate_repository(&root, &document, &configuration, &metadata).is_empty());
    }

    #[test]
    fn invalid_fixture_rejects_directory_wide_exception() {
        let root = fixture("invalid-directory-exception");
        let configuration: ArchitectureConfiguration =
            read_configuration(&root).expect("syntactically valid fixture configuration");
        let error = validate_exact_scope(&root, &configuration.exceptions[0].scope)
            .expect_err("directory scope must fail");
        assert_eq!(error, "directory-wide exceptions are forbidden");
    }

    #[test]
    fn invalid_fixture_produces_linked_rule_diagnostic() {
        let root = fixture("invalid-directory-exception");
        let configuration = read_configuration(&root).expect("fixture configuration parses");
        let metadata = CargoMetadata {
            packages: Vec::new(),
            workspace_members: Vec::new(),
            workspace_root: root.clone(),
        };
        let diagnostics = validate_repository(&root, "", &configuration, &metadata);
        let rendered = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.rule_id == "ARCH-EXCEPTION-FORMAT")
            .expect("exception diagnostic")
            .render();
        assert!(rendered.contains("[ARCH-EXCEPTION-FORMAT]"));
        assert!(rendered.contains("ARCHITECTURE.md#architecture-rule-index"));
    }

    #[test]
    fn rule_index_rejects_unknown_and_duplicate_rows() {
        let root = fixture("valid");
        let configuration = read_configuration(&root).expect("fixture configuration parses");
        let document = format!(
            "{}\n| `ARCH-RULE-REGISTRY` | Structural / harness | Duplicate. | `cargo dev check architecture` | Remove it. |\n| `ARCH-NOT-REGISTERED` | Structural / migration-gated | Unknown. | `cargo dev check architecture` | Register it. |",
            valid_document(&configuration)
        );
        let metadata = CargoMetadata {
            packages: Vec::new(),
            workspace_members: Vec::new(),
            workspace_root: root.clone(),
        };
        let diagnostics = validate_repository(&root, &document, &configuration, &metadata);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("ARCH-RULE-REGISTRY more than once")
        }));
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("ARCH-NOT-REGISTERED"))
        );
    }

    #[test]
    fn invalid_fixture_rejects_workspace_wide_exception() {
        let root = fixture("invalid-workspace-exception");
        let configuration: ArchitectureConfiguration =
            read_configuration(&root).expect("syntactically valid fixture configuration");
        assert!(validate_exact_scope(&root, &configuration.exceptions[0].scope).is_err());
    }

    #[test]
    fn cargo_metadata_fixture_rejects_a_missing_member_record() {
        let root = fixture("valid");
        let metadata: CargoMetadata = serde_json::from_str(&format!(
            r#"{{"packages":[],"workspace_members":["missing"],"workspace_root":{}}}"#,
            serde_json::to_string(&root).expect("path serializes")
        ))
        .expect("metadata fixture parses");
        let mut diagnostics = Vec::<Diagnostic>::new();
        validate_metadata(&root, &metadata, &mut diagnostics);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.rule_id == "ARCH-CARGO-METADATA"
                && diagnostic.message.contains("no package record")
        }));
    }

    #[test]
    fn invalid_exception_fixture_covers_accountability_and_exact_scope_failures() {
        let root = fixture("invalid-exceptions");
        let configuration = read_configuration(&root).expect("fixture configuration parses");
        let metadata = CargoMetadata {
            packages: Vec::new(),
            workspace_members: Vec::new(),
            workspace_root: root.clone(),
        };
        let diagnostics = validate_repository(&root, "", &configuration, &metadata);
        let messages = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.rule_id == "ARCH-EXCEPTION-FORMAT")
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>();
        for expected in [
            "unknown rule",
            "expiry or tracking task",
            "globs are forbidden",
            "beyond the end",
            "item does not exist",
            "has expired",
            "duplicates exact scope",
            "unresolved tracking task",
        ] {
            assert!(
                messages.iter().any(|message| message.contains(expected)),
                "missing diagnostic containing {expected}"
            );
        }
    }

    #[test]
    fn missing_required_exception_field_is_rejected_during_parsing() {
        let root = fixture("invalid-missing-field");
        let error = read_configuration(&root).expect_err("missing owner must fail");
        assert!(error.to_string().contains("missing field `owner`"));
    }

    #[test]
    fn cargo_metadata_fixture_rejects_repository_escaping_path_dependency() {
        let root = fixture("valid");
        let package_id = "fixture 0.1.0".to_owned();
        let metadata = CargoMetadata {
            packages: vec![CargoPackage {
                id: package_id.clone(),
                name: "fixture".into(),
                manifest_path: root.join("src/lib.rs"),
                metadata: Value::Null,
                dependencies: vec![CargoDependency {
                    name: "escape".into(),
                    path: Some(Path::new("/tmp").to_path_buf()),
                    kind: None,
                }],
            }],
            workspace_members: vec![package_id],
            workspace_root: root.clone(),
        };
        let mut diagnostics = Vec::new();
        validate_metadata(&root, &metadata, &mut diagnostics);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.rule_id == "ARCH-CARGO-METADATA"
                && diagnostic.message.contains("repository-escaping")
        }));
    }

    #[test]
    fn impossible_calendar_expiry_is_rejected() {
        assert!(!super::date_shape("2026-02-29"));
        assert!(!super::date_shape("2026-13-01"));
        assert!(super::date_shape("2028-02-29"));
    }
}
