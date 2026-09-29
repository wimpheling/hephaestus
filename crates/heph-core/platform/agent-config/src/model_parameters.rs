//! Typed parameter and secret declarations.
use super::model_runtime::{SecretDeliveryMode, SecretPhase};
use serde::{Deserialize, Serialize};
/// Typed parameter declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterDeclaration {
    /// Stable bounded name.
    pub name: String,
    /// Type and explicit bounds.
    #[serde(flatten)]
    pub value_type: ParameterType,
    /// Whether a value or default is required.
    #[serde(default)]
    pub required: bool,
    /// Optional TOML default.
    #[serde(default)]
    pub default: Option<ParameterDefault>,
    /// Whether UI and telemetry redact the ordinary value.
    #[serde(default)]
    pub sensitive: bool,
}

/// Bounded parameter type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParameterType {
    /// UTF-8 string bounds.
    String {
        /// Inclusive minimum characters.
        minimum_length: u16,
        /// Inclusive maximum characters.
        maximum_length: u16,
    },
    /// Signed integer bounds.
    Integer {
        /// Inclusive minimum.
        minimum: i64,
        /// Inclusive maximum.
        maximum: i64,
    },
    /// Boolean.
    Boolean,
    /// Exact bounded choices.
    Enum {
        /// Allowed strings.
        values: Vec<String>,
    },
}

/// Supported TOML default values without coercion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParameterDefault {
    /// String or enum default.
    String(String),
    /// Integer default.
    Integer(i64),
    /// Boolean default.
    Boolean(bool),
}

/// Symbolic secret declaration without tenant identifiers or values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretSlotDeclaration {
    /// Stable symbolic key.
    pub key: String,
    /// Human-readable non-secret purpose.
    pub purpose: String,
    /// Whether a runnable instance revision requires a binding.
    #[serde(default)]
    pub required: bool,
    /// Accepted delivery modes.
    pub delivery_modes: Vec<SecretDeliveryMode>,
    /// Accepted execution phases.
    pub phases: Vec<SecretPhase>,
    /// Optional exact broker destination ceiling.
    #[serde(default)]
    pub destinations: Vec<String>,
}
