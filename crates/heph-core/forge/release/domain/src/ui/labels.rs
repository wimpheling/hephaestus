//! Validated user-facing UI labels.

use crate::ReleaseValueError;
use serde::{Deserialize, Serialize};
use std::fmt;

/// A bounded user-facing UI label.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct UiLabel(String);

impl UiLabel {
    /// Trims and validates a nonempty label of at most 80 Unicode characters.
    ///
    /// # Errors
    ///
    /// Returns [`ReleaseValueError::InvalidUiLabel`] for empty, oversized, or
    /// control-containing input.
    pub fn parse(value: impl Into<String>) -> Result<Self, ReleaseValueError> {
        let value = value.into();
        let contains_forbidden_character = value.chars().any(|character| {
            character.is_control()
                || is_bidi_control(character)
                || matches!(character, '\u{2028}' | '\u{2029}')
        });
        let trimmed = value.trim();
        let valid =
            !contains_forbidden_character && !trimmed.is_empty() && trimmed.chars().count() <= 80;
        if valid {
            Ok(Self(trimmed.to_owned()))
        } else {
            Err(ReleaseValueError::InvalidUiLabel)
        }
    }

    /// Returns the trimmed validated label.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

impl fmt::Display for UiLabel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for UiLabel {
    type Error = ReleaseValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<UiLabel> for String {
    fn from(value: UiLabel) -> Self {
        value.0
    }
}
