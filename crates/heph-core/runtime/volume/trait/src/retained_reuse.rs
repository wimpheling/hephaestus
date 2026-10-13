//! Read-only comparison contract for positively retained completed owned births.

use crate::{OwnedProvisioningClaim, OwnedReadyRetentionContext, VolumeError};
use async_trait::async_trait;
use uuid::Uuid;

/// Exact retained comparison data; never physical custody or current actor authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedRetainedReuseContext {
    /// Full original purpose, operation, Ready observation and closed comparison pins.
    pub retained: OwnedReadyRetentionContext,
    /// Actual positive retention transition operation, not necessarily the first Remove.
    pub retention_operation_id: Uuid,
}
impl OwnedRetainedReuseContext {
    /// Checks the full original context and the distinct positive disposition identity.
    ///
    /// Backend reads must independently qualify committed retention and completed
    /// original Create; constructing this value cannot grant reuse authority.
    ///
    /// # Errors
    /// Foreign original context, nil disposition or the original Create attempt denies.
    pub fn validate(&self) -> Result<(), VolumeError> {
        self.retained.validate()?;
        if self.retention_operation_id.is_nil()
            || self.retention_operation_id
                == self
                    .retained
                    .original
                    .claim
                    .purpose
                    .receipt()
                    .intent
                    .creation()
                    .fields()
                    .attempt_id
        {
            return Err(VolumeError::IntentConflict);
        }
        Ok(())
    }
}

/// Worker-only read boundary for retained reuse, separate from unfinished Remove.
#[async_trait]
pub trait VolumeRetainedReuseRepository: Send + Sync + 'static {
    /// Reads one completed positive retention under unchanged global quiescence.
    ///
    /// The expected operation comes from exact committed retention discovery; it
    /// need not equal the closure's first Remove operation. Disk owner UUID/root
    /// and VM namespace pins remain separate. This read allocates no generation,
    /// writes no fact, and grants neither mount/write nor reopening authority.
    /// Call under actual Local custody and recheck before returning that custody.
    ///
    /// # Errors
    /// Unsupported adapters, foreign pins, unfinished Create, absent positive
    /// retention, reopened heads or uncertain lease/consumer history deny.
    async fn retained_reuse_context(
        &self,
        _claim: &OwnedProvisioningClaim,
        _closure_id: Uuid,
        _retention_operation_id: Uuid,
    ) -> Result<OwnedRetainedReuseContext, VolumeError> {
        Err(VolumeError::InvalidState("retained reuse is unsupported"))
    }
}
