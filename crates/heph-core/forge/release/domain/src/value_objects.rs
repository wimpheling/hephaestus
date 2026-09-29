use forge_domain::GitRef;
use serde::{Deserialize, Serialize};
use std::{fmt, path::Path};

use super::ReleaseValueError;

macro_rules! bounded_key {
    ($name:ident, $documentation:literal, $maximum:expr) => {
        #[doc = $documentation]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Parses a bounded lowercase key.
            ///
            /// # Errors
            ///
            /// Returns [`ReleaseValueError::InvalidKey`] for malformed input.
            pub fn parse(value: impl Into<String>) -> Result<Self, ReleaseValueError> {
                let value = value.into();
                let valid = (1..=$maximum).contains(&value.len())
                    && value.bytes().enumerate().all(|(index, byte)| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || ((byte == b'_' || byte == b'-') && index > 0)
                    });
                if !valid {
                    return Err(ReleaseValueError::InvalidKey {
                        kind: stringify!($name),
                    });
                }
                Ok(Self(value))
            }

            /// Returns the validated value.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl TryFrom<String> for $name {
            type Error = ReleaseValueError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

bounded_key!(
    AgentKey,
    "A stable exported key unique within a source repository.",
    64
);
bounded_key!(
    ParameterName,
    "A stable typed-parameter name owned by a release agent.",
    64
);
bounded_key!(
    InstanceName,
    "A project-scoped reusable agent instance name.",
    128
);

/// A normalized release version selected by its source repository.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ReleaseVersion(String);

impl ReleaseVersion {
    /// Parses a printable version with no path or surrounding whitespace.
    ///
    /// # Errors
    ///
    /// Returns [`ReleaseValueError::InvalidVersion`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, ReleaseValueError> {
        let value = value.into();
        if !(1..=128).contains(&value.len())
            || value.trim() != value
            || value.contains(['/', '\\'])
            || value
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
        {
            return Err(ReleaseValueError::InvalidVersion);
        }
        Ok(Self(value))
    }

    /// Returns the normalized version.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ReleaseVersion {
    type Error = ReleaseValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<ReleaseVersion> for String {
    fn from(value: ReleaseVersion) -> Self {
        value.0
    }
}

/// A normalized relative artifact path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ArtifactPath(String);

impl ArtifactPath {
    /// Validates a path independently from any host filesystem.
    ///
    /// # Errors
    ///
    /// Returns [`ReleaseValueError::InvalidArtifactPath`] for absolute,
    /// traversal, empty, reserved, or oversized paths.
    pub fn parse(value: impl Into<String>) -> Result<Self, ReleaseValueError> {
        let value = value.into();
        let path = Path::new(&value);
        let valid = (1..=1024).contains(&value.len())
            && !path.is_absolute()
            && !value.contains('\\')
            && path.components().all(|component| {
                matches!(
                    component,
                    std::path::Component::Normal(part)
                        if part != ".git" && !part.to_string_lossy().is_empty()
                )
            });
        if !valid {
            return Err(ReleaseValueError::InvalidArtifactPath);
        }
        Ok(Self(value))
    }

    /// Returns the normalized slash-separated path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ArtifactPath {
    type Error = ReleaseValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<ArtifactPath> for String {
    fn from(value: ArtifactPath) -> Self {
        value.0
    }
}

/// An exact ref or bounded ref-prefix selector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum RefSelector {
    /// Match one exact fully-qualified ref.
    Exact(GitRef),
    /// Match descendants of a fully-qualified prefix.
    Prefix(GitRef),
}

impl RefSelector {
    /// Parses `refs/...` as exact or a terminal `/*` as a prefix.
    ///
    /// # Errors
    ///
    /// Returns [`ReleaseValueError::InvalidRefSelector`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, ReleaseValueError> {
        let value = value.into();
        if let Some(prefix) = value.strip_suffix("/*") {
            return GitRef::parse(prefix.to_owned())
                .map(Self::Prefix)
                .map_err(|_| ReleaseValueError::InvalidRefSelector);
        }
        GitRef::parse(value)
            .map(Self::Exact)
            .map_err(|_| ReleaseValueError::InvalidRefSelector)
    }

    /// Returns whether a repository update matches this selector.
    #[must_use]
    pub fn matches(&self, git_ref: &GitRef) -> bool {
        match self {
            Self::Exact(expected) => expected == git_ref,
            Self::Prefix(prefix) => git_ref
                .as_str()
                .strip_prefix(prefix.as_str())
                .is_some_and(|suffix| suffix.starts_with('/')),
        }
    }
}
