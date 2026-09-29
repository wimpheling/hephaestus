//! Durable, redacted failure reporting for long-lived gateway services.

use async_trait::async_trait;
use std::fmt;

use crate::{GatewayServiceInstanceLease, GatewayServiceOwner};

/// Maximum signed value accepted for a provider exit code or signal.
pub const MAX_SERVICE_EXIT_VALUE: i32 = 255;

/// Stable redacted category for a service launch or runtime failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayServiceFailureCode {
    /// Artifact or configuration preparation failed.
    Preparation,
    /// Starting the provider instance failed.
    Startup,
    /// The service did not become ready.
    Readiness,
    /// A health check failed after readiness.
    Health,
    /// The provider reported an unexpected process exit.
    UnexpectedExit,
    /// Provider or materializer cleanup failed.
    Cleanup,
}

impl GatewayServiceFailureCode {
    /// Returns the bounded database representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Preparation => "preparation",
            Self::Startup => "startup",
            Self::Readiness => "readiness",
            Self::Health => "health",
            Self::UnexpectedExit => "unexpected_exit",
            Self::Cleanup => "cleanup",
        }
    }
}

impl fmt::Display for GatewayServiceFailureCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One redacted service failure report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayServiceFailure {
    /// Bounded failure category.
    pub code: GatewayServiceFailureCode,
    /// Process exit code, when the provider supplied one.
    pub exit_code: Option<i32>,
    /// Process signal, when the provider supplied one.
    pub exit_signal: Option<i32>,
}

impl GatewayServiceFailure {
    /// Constructs a failure report and validates provider exit values.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceFailureStoreError::InvalidArgument`] when both
    /// exit forms are present, a value is outside the bounded provider range,
    /// or exit details accompany a non-exit failure.
    pub fn new(
        code: GatewayServiceFailureCode,
        exit_code: Option<i32>,
        exit_signal: Option<i32>,
    ) -> Result<Self, GatewayServiceFailureStoreError> {
        let failure = Self {
            code,
            exit_code,
            exit_signal,
        };
        failure.validate()?;
        Ok(failure)
    }

    /// Validates the report before it crosses a storage boundary.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceFailureStoreError::InvalidArgument`] for an
    /// invalid exit shape or value.
    pub fn validate(self) -> Result<(), GatewayServiceFailureStoreError> {
        if (self.exit_code.is_some() && self.exit_signal.is_some())
            || (self.code != GatewayServiceFailureCode::UnexpectedExit
                && (self.exit_code.is_some() || self.exit_signal.is_some()))
            || self
                .exit_code
                .is_some_and(|value| !(0..=MAX_SERVICE_EXIT_VALUE).contains(&value))
            || self
                .exit_signal
                .is_some_and(|value| !(1..=MAX_SERVICE_EXIT_VALUE).contains(&value))
        {
            return Err(GatewayServiceFailureStoreError::InvalidArgument);
        }
        Ok(())
    }
}

/// Errors returned by durable service failure reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GatewayServiceFailureStoreError {
    /// The report, owner, or instance identity is malformed.
    #[error("invalid gateway service failure argument")]
    InvalidArgument,
    /// The instance no longer belongs to the exact live owner.
    #[error("gateway service failure owner is stale")]
    StaleLease,
    /// Durable storage was unavailable.
    #[error("gateway service failure storage is unavailable")]
    Unavailable,
}

/// Provider-neutral durable failure reporting for one exact service instance.
#[async_trait]
pub trait GatewayServiceFailureStore: Send + Sync {
    /// Records one failure for an exact live owner and lease.
    async fn record_failure(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
        failure: GatewayServiceFailure,
    ) -> Result<(), GatewayServiceFailureStoreError>;
}
