use gateway_domain::HttpMethod;

/// Raw canonical absolute HTTP path used for adapter-side declaration
/// classification. Its bound covers a 256-byte route base plus a 256-byte
/// published file path and their separating slash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiBrowserHttpPath(String);

impl UiBrowserHttpPath {
    /// Parses the path grammar used by the HTTP serving boundary.
    ///
    /// # Errors
    ///
    /// Returns [`UiBrowserHttpPathError::Invalid`] when the path is not a
    /// canonical absolute UI path.
    pub fn parse(value: impl Into<String>) -> Result<Self, UiBrowserHttpPathError> {
        let value = value.into();
        let valid = (2..=514).contains(&value.len())
            && value.starts_with('/')
            && !value.ends_with('/')
            && value.bytes().all(is_ascii_unreserved_or_slash)
            && value[1..]
                .split('/')
                .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
        if valid {
            Ok(Self(value))
        } else {
            Err(UiBrowserHttpPathError::Invalid)
        }
    }

    /// Returns the validated absolute path without a query or fragment.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const fn is_ascii_unreserved_or_slash(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/')
}

/// Invalid raw HTTP authority path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UiBrowserHttpPathError {
    /// Path is not a canonical absolute ASCII UI path.
    #[error("invalid UI HTTP path")]
    Invalid,
}

/// Raw canonical HTTP request used for adapter-side declaration classification.
/// Query text is handled by the HTTP boundary and is excluded from this
/// authority port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiBrowserHttpRequest {
    /// Canonical method vocabulary.
    pub method: HttpMethod,
    /// Canonical absolute path without query.
    path: UiBrowserHttpPath,
}

impl UiBrowserHttpRequest {
    /// Creates one raw request after the HTTP boundary has validated its
    /// canonical path grammar.
    ///
    /// # Errors
    ///
    /// Returns [`UiBrowserHttpPathError::Invalid`] when `path` is not a
    /// canonical absolute UI path.
    pub fn new(
        method: HttpMethod,
        path: impl Into<String>,
    ) -> Result<Self, UiBrowserHttpPathError> {
        Ok(Self {
            method,
            path: UiBrowserHttpPath::parse(path)?,
        })
    }

    /// Returns the canonical method.
    #[must_use]
    pub const fn method(&self) -> HttpMethod {
        self.method
    }

    /// Returns the canonical absolute path.
    #[must_use]
    pub const fn path(&self) -> &UiBrowserHttpPath {
        &self.path
    }
}

/// The declaration kind selected by the raw HTTP classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiGatewayRequestKind {
    /// A route-base/descendant request for a managed service.
    Managed,
    /// An exact declared API route and method.
    Api,
}

/// Safe managed/API request shape after child authentication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiGatewayRequestProjection {
    /// The exact declaration kind selected by the adapter.
    pub kind: UiGatewayRequestKind,
    /// Validated raw HTTP path, including the managed route base when present.
    /// The gateway worker derives the current binding and route again.
    pub path: UiBrowserHttpPath,
    /// The canonical HTTP method selected by the declaration matcher.
    pub method: HttpMethod,
}
