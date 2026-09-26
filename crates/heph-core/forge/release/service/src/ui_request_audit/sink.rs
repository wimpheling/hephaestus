use async_trait::async_trait;
use std::fmt;
use thiserror::Error;

use super::{NewUiRequestAuditEvent, UiRequestAuditSurface};

/// Provider-neutral sink for one immutable UI request audit event.
#[async_trait]
pub trait UiRequestAuditSink: Send + Sync {
    /// Appends one event in its own transaction.
    async fn append(&self, event: NewUiRequestAuditEvent) -> Result<(), UiRequestAuditError>;
}

/// Errors from the audit persistence boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UiRequestAuditError {
    /// The audit store could not commit the event.
    #[error("UI request audit persistence is unavailable")]
    Unavailable,
}

impl fmt::Display for UiRequestAuditSurface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
