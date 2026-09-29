use std::fmt;

use serde::{Deserialize, Serialize};

use super::{
    MAX_HEADER_NAME_BYTES, MAX_HEADER_VALUE_BYTES, MAX_ROUTE_BYTES, MAX_TRACE_CONTEXT_BYTES,
    errors::MailboxDomainError, identifiers::validate_opaque,
};

/// A bounded absolute route without a URI scheme, authority, or fragment.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EnvelopeRoute(String);

impl EnvelopeRoute {
    /// Parses one bounded relative route.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::InvalidRoute`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, MailboxDomainError> {
        let value = value.into();
        if !(1..=MAX_ROUTE_BYTES).contains(&value.len())
            || !value.starts_with('/')
            || value.starts_with("//")
            || value.contains('#')
            || value
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
        {
            return Err(MailboxDomainError::InvalidRoute);
        }
        Ok(Self(value))
    }

    /// Returns the validated route.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EnvelopeRoute {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
impl TryFrom<String> for EnvelopeRoute {
    type Error = MailboxDomainError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}
impl From<EnvelopeRoute> for String {
    fn from(value: EnvelopeRoute) -> Self {
        value.0
    }
}

/// A lower-case selected HTTP header name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SelectedHeaderName(String);

impl SelectedHeaderName {
    /// Parses a lower-case ASCII token name.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::InvalidHeaderName`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, MailboxDomainError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_HEADER_NAME_BYTES
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(MailboxDomainError::InvalidHeaderName);
        }
        Ok(Self(value))
    }

    /// Returns the normalized header name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SelectedHeaderName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
impl TryFrom<String> for SelectedHeaderName {
    type Error = MailboxDomainError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}
impl From<SelectedHeaderName> for String {
    fn from(value: SelectedHeaderName) -> Self {
        value.0
    }
}

/// One bounded selected header value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SelectedHeaderValue(String);

impl SelectedHeaderValue {
    /// Parses a bounded header value without control characters.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::InvalidHeaderValue`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, MailboxDomainError> {
        let value = value.into();
        if value.len() > MAX_HEADER_VALUE_BYTES || value.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(MailboxDomainError::InvalidHeaderValue);
        }
        Ok(Self(value))
    }

    /// Returns the validated header value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SelectedHeaderValue {
    type Error = MailboxDomainError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}
impl From<SelectedHeaderValue> for String {
    fn from(value: SelectedHeaderValue) -> Self {
        value.0
    }
}
/// A bounded opaque distributed trace context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TraceContext(String);

impl TraceContext {
    /// Parses a bounded opaque trace context.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::InvalidTraceContext`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, MailboxDomainError> {
        let value = value.into();
        validate_opaque(
            &value,
            MAX_TRACE_CONTEXT_BYTES,
            MailboxDomainError::InvalidTraceContext,
        )?;
        Ok(Self(value))
    }

    /// Returns the validated trace context.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for TraceContext {
    type Error = MailboxDomainError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}
impl From<TraceContext> for String {
    fn from(value: TraceContext) -> Self {
        value.0
    }
}
