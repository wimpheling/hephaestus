//! Provider decisions and typed authorization failures.

use serde::{Deserialize, Serialize};
use std::error::Error;

/// Result returned by an authorization provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorizationDecision {
    /// The operation is permitted.
    Allow,
    /// The operation is denied.
    Deny,
}

impl AuthorizationDecision {
    /// Returns whether the decision permits the operation.
    #[must_use]
    pub const fn is_allowed(self) -> bool {
        matches!(self, Self::Allow)
    }
}

/// Typed authorization failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AuthzError {
    /// Actor context was not set on the transaction.
    #[error("authenticated actor context is missing")]
    MissingActorContext,
    /// An object type was not recognized.
    #[error("unknown authorization object type {0:?}")]
    UnknownObjectType(String),
    /// A permission was not recognized.
    #[error("unknown authorization permission {0:?}")]
    UnknownPermission(String),
    /// A subject or object identifier was malformed.
    #[error("malformed authorization identifier {0:?}")]
    MalformedId(String),
    /// The configured authorization evaluator failed.
    #[error("authorization evaluator failed: {0}")]
    Evaluator(#[source] Box<dyn Error + Send + Sync>),
}

impl AuthzError {
    /// Wraps a provider-specific evaluator failure without exposing its type.
    #[must_use]
    pub fn evaluator(error: impl Error + Send + Sync + 'static) -> Self {
        Self::Evaluator(Box::new(error))
    }
}
