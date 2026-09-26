//! Trigger, hash, diagnostic, and parsed-result declarations.
use super::model_build::ResourceLimits;
use crate::{AgentConfig, RepositoryGatewaysConfig, RepositoryOciImagesConfig};
use forge_domain::GitRef;
use serde::{Deserialize, Serialize};

/// Declared secret delivery mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretDeliveryMode {
    /// Ephemeral raw-file delivery.
    Raw,
    /// Non-disclosing broker use.
    Brokered,
}

/// Declared secret execution phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretPhase {
    /// Ordinary run.
    Normal,
    /// Candidate update hook.
    Update,
}

/// Candidate-release update hook executed only inside an isolated guest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateHookConfig {
    /// Executable relative to `/release`.
    pub command: String,
    /// Fixed arguments, without a host shell.
    #[serde(default)]
    pub arguments: Vec<String>,
    /// Wall-clock timeout.
    pub timeout_seconds: u32,
    /// Hook-specific resources within the release ceiling.
    pub resources: ResourceLimits,
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
    /// Constructs a hash from its normalized hexadecimal representation.
    #[must_use]
    pub const fn from_value(value: String) -> Self {
        Self(value)
    }
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
    /// Hash of deterministic normalized configuration serialization.
    pub normalized_hash: Option<ConfigHash>,
    /// Validated configuration, absent on failure.
    pub config: Option<AgentConfig>,
    /// Structured parser and validation diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// Result of parsing one repository `heph.images.toml` manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRepositoryOciImages {
    /// Hash of the exact source bytes.
    pub hash: ConfigHash,
    /// Validated manifest, absent on failure.
    pub config: Option<RepositoryOciImagesConfig>,
    /// Structured parser and validation diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// Result of parsing one repository `heph.gateways.toml` manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedRepositoryGateways {
    /// Hash of the exact source bytes.
    pub hash: ConfigHash,
    /// Hash of deterministic normalized configuration serialization.
    pub normalized_hash: Option<ConfigHash>,
    /// Validated manifest, absent on failure.
    pub config: Option<RepositoryGatewaysConfig>,
    /// Structured parser and validation diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}
