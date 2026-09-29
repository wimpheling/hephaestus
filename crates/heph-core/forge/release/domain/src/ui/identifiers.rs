//! Validated UI keys and route paths.

use crate::ReleaseValueError;
use serde::{Deserialize, Serialize};
use std::fmt;

/// A stable lowercase UI key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct UiKey(String);

impl UiKey {
    /// Parses a key beginning with a lowercase ASCII letter and followed by
    /// lowercase ASCII letters, digits, or hyphens.
    ///
    /// # Errors
    ///
    /// Returns [`ReleaseValueError::InvalidUiKey`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, ReleaseValueError> {
        let value = value.into();
        let mut bytes = value.bytes();
        let valid = (1..=64).contains(&value.len())
            && bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
            && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if valid {
            Ok(Self(value))
        } else {
            Err(ReleaseValueError::InvalidUiKey)
        }
    }

    /// Returns the validated key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for UiKey {
    type Error = ReleaseValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<UiKey> for String {
    fn from(value: UiKey) -> Self {
        value.0
    }
}

/// A platform-relative UI route path made of ASCII unreserved segments.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct UiRoutePath(String);

impl UiRoutePath {
    /// Parses a relative path without URL syntax or traversal segments.
    ///
    /// # Errors
    ///
    /// Returns [`ReleaseValueError::InvalidUiRoutePath`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, ReleaseValueError> {
        let value = value.into();
        let valid = (1..=256).contains(&value.len())
            && value.bytes().all(is_ascii_unreserved_or_slash)
            && !value.starts_with('/')
            && !value.ends_with('/')
            && value
                .split('/')
                .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
        if valid {
            Ok(Self(value))
        } else {
            Err(ReleaseValueError::InvalidUiRoutePath)
        }
    }

    /// Returns the validated relative path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const fn is_ascii_unreserved_or_slash(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/')
}

impl fmt::Display for UiRoutePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for UiRoutePath {
    type Error = ReleaseValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<UiRoutePath> for String {
    fn from(value: UiRoutePath) -> Self {
        value.0
    }
}
