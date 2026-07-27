//! Versioned `agent.toml` parsing, validation, and trigger matching.

use forge_domain::GitRef;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt::Write as _, path::Path};

/// The only configuration version supported by this phase.
pub const SUPPORTED_VERSION: u32 = 1;

/// A successfully parsed and validated configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Configuration schema version.
    pub version: u32,
    /// Agent identity.
    pub agent: AgentIdentity,
    /// Guest process.
    pub guest: GuestConfig,
    /// Compute limits.
    pub resources: ResourceLimits,
    /// Immutable guest root image.
    pub root_image: RootImage,
    /// Repository workspace mount intent.
    pub workspace: WorkspaceMount,
    /// Persistent agent-state volume intent.
    pub state_volume: StateVolume,
    /// Guest networking profile.
    pub network: NetworkConfig,
    /// Run trigger policy.
    pub triggers: TriggerConfig,
}

/// Human-readable agent identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentIdentity {
    /// Name unique within a repository.
    pub name: String,
}

/// Guest process definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestConfig {
    /// Absolute guest executable path.
    pub command: String,
    /// Arguments excluding the executable.
    #[serde(default)]
    pub arguments: Vec<String>,
    /// Absolute guest working directory.
    pub working_directory: String,
}

/// Compute resource limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Virtual CPU count.
    pub vcpus: u8,
    /// Guest memory in mebibytes.
    pub memory_mib: u32,
}

/// Root image selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootImage {
    /// Immutable image reference, normally digest-pinned.
    pub reference: String,
}

/// Repository workspace mount intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMount {
    /// Whether the exact received tree is mounted in the guest.
    pub mount: bool,
    /// Guest mount path.
    pub path: String,
    /// Whether the mount must be read-only.
    #[serde(default = "default_true")]
    pub read_only: bool,
}

/// Persistent state-volume selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateVolume {
    /// Whether the agent receives its persistent state volume.
    pub enabled: bool,
}

/// Guest network selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Named provider-neutral network profile.
    pub profile: NetworkProfile,
}

/// Provider-neutral network profiles supported in the initial schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkProfile {
    /// No guest network.
    Disabled,
    /// Egress-capable user-mode network.
    Egress,
}

/// Push-trigger selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerConfig {
    /// Whether accepted pushes may create runs.
    pub push: bool,
    /// Fully-qualified refs or terminal `/*` prefixes that may trigger.
    #[serde(default)]
    pub refs: Vec<String>,
}

impl TriggerConfig {
    /// Returns whether an accepted update to `git_ref` triggers a run.
    #[must_use]
    pub fn matches(&self, git_ref: &GitRef) -> bool {
        self.push
            && (self.refs.is_empty()
                || self.refs.iter().any(|pattern| {
                    pattern.strip_suffix("/*").map_or_else(
                        || pattern == git_ref.as_str(),
                        |prefix| {
                            git_ref
                                .as_str()
                                .strip_prefix(prefix)
                                .is_some_and(|suffix| suffix.starts_with('/'))
                        },
                    )
                }))
    }
}

/// Stable hash of the original configuration bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConfigHash(String);

impl ConfigHash {
    /// Returns the lowercase SHA-256 digest.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One structured configuration diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Stable diagnostic code.
    pub code: String,
    /// Field path, when the error belongs to one field.
    pub path: Option<String>,
    /// Human-readable explanation.
    pub message: String,
}

/// Result of parsing and validating one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedConfig {
    /// Hash of the exact source bytes.
    pub hash: ConfigHash,
    /// Validated configuration, absent on failure.
    pub config: Option<AgentConfig>,
    /// Structured parser and validation diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// Parses and validates `agent.toml`.
#[must_use]
pub fn parse(source: &[u8]) -> ParsedConfig {
    let hash = hash(source);
    let text = match std::str::from_utf8(source) {
        Ok(text) => text,
        Err(error) => {
            return ParsedConfig {
                hash,
                config: None,
                diagnostics: vec![Diagnostic {
                    code: String::from("invalid_utf8"),
                    path: None,
                    message: error.to_string(),
                }],
            };
        }
    };
    let config = match toml::from_str::<AgentConfig>(text) {
        Ok(config) => config,
        Err(error) => {
            return ParsedConfig {
                hash,
                config: None,
                diagnostics: vec![Diagnostic {
                    code: String::from("invalid_toml"),
                    path: error
                        .span()
                        .map(|span| format!("bytes {}..{}", span.start, span.end)),
                    message: error.message().to_owned(),
                }],
            };
        }
    };
    let diagnostics = validate(&config);
    ParsedConfig {
        hash,
        config: diagnostics.is_empty().then_some(config),
        diagnostics,
    }
}

fn hash(source: &[u8]) -> ConfigHash {
    let digest = Sha256::digest(source);
    let mut value = String::with_capacity(64);
    for byte in digest {
        write!(&mut value, "{byte:02x}").expect("writing to a String cannot fail");
    }
    ConfigHash(value)
}

fn validate(config: &AgentConfig) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if config.version != SUPPORTED_VERSION {
        diagnostic(
            &mut diagnostics,
            "unsupported_version",
            "version",
            format!(
                "configuration version {} is unsupported; expected {SUPPORTED_VERSION}",
                config.version
            ),
        );
    }
    if config.agent.name.trim().is_empty() || config.agent.name.len() > 128 {
        diagnostic(
            &mut diagnostics,
            "invalid_agent_name",
            "agent.name",
            "name must contain 1 to 128 characters",
        );
    }
    validate_absolute_path(
        &mut diagnostics,
        "guest.command",
        &config.guest.command,
        "invalid_guest_command",
    );
    validate_absolute_path(
        &mut diagnostics,
        "guest.working_directory",
        &config.guest.working_directory,
        "invalid_working_directory",
    );
    if !(1..=64).contains(&config.resources.vcpus) {
        diagnostic(
            &mut diagnostics,
            "invalid_vcpus",
            "resources.vcpus",
            "vcpus must be between 1 and 64",
        );
    }
    if !(128..=1_048_576).contains(&config.resources.memory_mib) {
        diagnostic(
            &mut diagnostics,
            "invalid_memory",
            "resources.memory_mib",
            "memory_mib must be between 128 and 1048576",
        );
    }
    if config.root_image.reference.trim().is_empty() {
        diagnostic(
            &mut diagnostics,
            "missing_root_image",
            "root_image.reference",
            "root image reference must not be empty",
        );
    }
    if config.workspace.mount {
        validate_absolute_path(
            &mut diagnostics,
            "workspace.path",
            &config.workspace.path,
            "invalid_workspace_path",
        );
    }
    for (index, pattern) in config.triggers.refs.iter().enumerate() {
        let value = pattern.strip_suffix("/*").unwrap_or(pattern);
        if GitRef::parse(value.to_owned()).is_err() {
            diagnostic(
                &mut diagnostics,
                "invalid_trigger_ref",
                format!("triggers.refs[{index}]"),
                "trigger must be a fully-qualified ref or end in /*",
            );
        }
    }
    diagnostics
}

fn validate_absolute_path(diagnostics: &mut Vec<Diagnostic>, path: &str, value: &str, code: &str) {
    let parsed = Path::new(value);
    if !parsed.is_absolute()
        || parsed
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        diagnostic(
            diagnostics,
            code,
            path,
            "path must be absolute and must not contain parent traversal",
        );
    }
}

fn diagnostic(
    diagnostics: &mut Vec<Diagnostic>,
    code: impl Into<String>,
    path: impl Into<String>,
    message: impl Into<String>,
) {
    diagnostics.push(Diagnostic {
        code: code.into(),
        path: Some(path.into()),
        message: message.into(),
    });
}

const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::{SUPPORTED_VERSION, parse};
    use forge_domain::GitRef;

    const VALID: &str = r#"
version = 1

[agent]
name = "reviewer"

[guest]
command = "/usr/bin/review"
arguments = ["--format=json"]
working_directory = "/workspace"

[resources]
vcpus = 2
memory_mib = 512

[root_image]
reference = "registry.example/agent@sha256:abc"

[workspace]
mount = true
path = "/workspace"
read_only = true

[state_volume]
enabled = true

[network]
profile = "disabled"

[triggers]
push = true
refs = ["refs/heads/*"]
"#;

    #[test]
    fn parses_and_matches_valid_v1() {
        let parsed = parse(VALID.as_bytes());
        let config = parsed.config.expect("valid config");
        assert_eq!(config.version, SUPPORTED_VERSION);
        assert!(parsed.diagnostics.is_empty());
        assert!(
            config
                .triggers
                .matches(&GitRef::parse("refs/heads/main").expect("valid ref"))
        );
        assert!(
            !config
                .triggers
                .matches(&GitRef::parse("refs/tags/v1").expect("valid ref"))
        );
        assert_eq!(parsed.hash.as_str().len(), 64);
    }

    #[test]
    fn reports_version_and_field_diagnostics() {
        let invalid = VALID
            .replace("version = 1", "version = 99")
            .replace("command = \"/usr/bin/review\"", "command = \"review\"");
        let parsed = parse(invalid.as_bytes());
        assert!(parsed.config.is_none());
        assert_eq!(parsed.diagnostics.len(), 2);
        assert_eq!(parsed.diagnostics[0].code, "unsupported_version");
        assert_eq!(parsed.diagnostics[1].path.as_deref(), Some("guest.command"));
    }

    #[test]
    fn reports_syntax_errors_without_panicking() {
        let parsed = parse(b"version = [");
        assert!(parsed.config.is_none());
        assert_eq!(parsed.diagnostics[0].code, "invalid_toml");
    }
}
