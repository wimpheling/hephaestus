use super::{PreparedInstanceDeleteRequest, PreparedInstanceDetachRequest};
use crate::{DeploymentError, PreparedInstanceSafetyObservation, PreparedInstanceSafetyState};
use uuid::Uuid;

/// Exact request-bound comparison of complete post-physical attachment evidence.
///
/// This closed shape is not a worker receipt or an authorization. The native
/// observer must independently qualify the represented original and full inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInstanceDetachObservation {
    request: PreparedInstanceDetachRequest,
    safety: PreparedInstanceSafetyObservation,
}
impl PreparedInstanceDetachObservation {
    /// Compares the whole target, positive Drained state and original removal ID.
    ///
    /// The supplied full inventory is retained unchanged, including terminal history;
    /// empty or host-filtered values do not themselves establish completeness.
    ///
    /// # Errors
    /// Rejects a substituted target/removal or any unproven/held safety state.
    pub fn new(
        request: PreparedInstanceDetachRequest,
        safety: PreparedInstanceSafetyObservation,
    ) -> Result<Self, DeploymentError> {
        if safety.target != *request.target()
            || safety.removal_id != Some(request.closure().removal_id)
        {
            return Err(DeploymentError::IntentMismatch);
        }
        if safety.state != PreparedInstanceSafetyState::Drained {
            return Err(DeploymentError::ReconciliationRequired);
        }
        Ok(Self { request, safety })
    }
    /// Returns every original request pin to compare before receipt recording.
    #[must_use]
    pub const fn request(&self) -> &PreparedInstanceDetachRequest {
        &self.request
    }
    /// Returns complete unchanged target/classification/inventory comparison data.
    #[must_use]
    pub const fn safety(&self) -> &PreparedInstanceSafetyObservation {
        &self.safety
    }
}

/// Exact request-bound committed tombstone comparison, never an Applied receipt.
///
/// The native reader must qualify actual terminal record/event/current publisher,
/// old Create/closure/committed Detach, Delete claim/config and complete history.
/// These pure fields alone prove none of those associations or physical effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInstanceDeleteObservation {
    request: PreparedInstanceDeleteRequest,
    terminal_operation_id: Uuid,
    after_instance_version: u64,
}
impl PreparedInstanceDeleteObservation {
    /// Checks a nonnil actual record identity and exact bounded successor version.
    ///
    /// A missing row or arbitrary positive version is not accepted. An expected
    /// bigint maximum cannot produce a representable successor and therefore denies.
    ///
    /// # Errors
    /// Rejects nil record identity, overflow or any version other than before CAS+1.
    pub fn new(
        request: PreparedInstanceDeleteRequest,
        terminal_operation_id: Uuid,
        after_instance_version: u64,
    ) -> Result<Self, DeploymentError> {
        let expected = request
            .expected_instance_version()
            .checked_add(1)
            .filter(|version| *version <= i64::MAX.cast_unsigned())
            .ok_or(DeploymentError::StaleClaim)?;
        if terminal_operation_id.is_nil() || after_instance_version != expected {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(Self {
            request,
            terminal_operation_id,
            after_instance_version,
        })
    }
    /// Returns full original and current comparison inputs to independently recheck.
    #[must_use]
    pub const fn request(&self) -> &PreparedInstanceDeleteRequest {
        &self.request
    }
    /// Returns actual immutable terminal operation identity, not an inferred UUID.
    #[must_use]
    pub const fn terminal_operation_id(&self) -> Uuid {
        self.terminal_operation_id
    }
    /// Returns the exact committed successor version, not an arbitrary newer CAS.
    #[must_use]
    pub const fn after_instance_version(&self) -> u64 {
        self.after_instance_version
    }
}
