use crate::ui::UiRoutePath;
use serde::{Deserialize, Serialize};
use std::fmt;

/// A validated platform-relative route selected by the UI declaration.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UiBrowserRoute(UiRoutePath);

impl UiBrowserRoute {
    /// Parses a route through the release UI path validator. This accepts no
    /// URL scheme, authority, query, fragment, absolute path, or traversal.
    ///
    /// # Errors
    ///
    /// Returns [`UiBrowserRouteError::Invalid`] for any non-relative route.
    pub fn parse(value: impl Into<String>) -> Result<Self, UiBrowserRouteError> {
        UiRoutePath::parse(value)
            .map(Self)
            .map_err(|_| UiBrowserRouteError::Invalid)
    }

    /// Returns the normalized validated path for routing within the bound UI.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Returns the underlying release-domain typed path.
    #[must_use]
    pub const fn path(&self) -> &UiRoutePath {
        &self.0
    }
}

impl TryFrom<String> for UiBrowserRoute {
    type Error = UiBrowserRouteError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<UiBrowserRoute> for String {
    fn from(value: UiBrowserRoute) -> Self {
        value.0.into()
    }
}

impl fmt::Display for UiBrowserRoute {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The route was not a safe platform-relative UI path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UiBrowserRouteError {
    /// The route contained URL syntax, traversal, or invalid bytes.
    #[error("UI browser route is invalid")]
    Invalid,
}
