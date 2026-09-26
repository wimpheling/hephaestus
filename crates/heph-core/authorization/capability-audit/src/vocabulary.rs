use super::CapabilityAuditError;
use std::fmt;

/// Kind of immutable capability evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityAuditEventKind {
    /// The runtime request was evaluated against immutable and live authority.
    AuthorizationDecision,
    /// An allowed capability invocation completed or failed.
    CapabilityUse,
}

impl CapabilityAuditEventKind {
    /// Returns the stable persistence representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AuthorizationDecision => "authorization_decision",
            Self::CapabilityUse => "capability_use",
        }
    }
}

/// Result of one capability authorization evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityDecision {
    /// The request is allowed to proceed.
    Allow,
    /// The request must not invoke the capability.
    Deny,
}

impl CapabilityDecision {
    /// Returns the stable persistence representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

/// Outcome of an already-authorized capability invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityUseOutcome {
    /// The controlled operation completed successfully.
    Succeeded,
    /// The controlled operation failed after authorization.
    Failed,
}

impl CapabilityUseOutcome {
    /// Returns the stable persistence representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

/// A bounded, non-sensitive machine reason suitable for durable audit rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityAuditReason(String);

impl CapabilityAuditReason {
    /// Parses a lower-snake-case reason code of at most 64 bytes.
    ///
    /// # Errors
    ///
    /// Rejects empty, oversized, or non-canonical values.
    pub fn parse(value: impl Into<String>) -> Result<Self, CapabilityAuditError> {
        let value = value.into();
        let mut bytes = value.bytes();
        let Some(first) = bytes.next() else {
            return Err(CapabilityAuditError::InvalidReasonCode);
        };
        if value.len() > 64
            || !first.is_ascii_lowercase()
            || bytes
                .any(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'))
        {
            return Err(CapabilityAuditError::InvalidReasonCode);
        }
        Ok(Self(value))
    }

    /// Returns the canonical reason code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CapabilityAuditReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
