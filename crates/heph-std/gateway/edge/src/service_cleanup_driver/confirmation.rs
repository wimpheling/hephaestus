use super::monitor::exact_claim;
use super::{
    GatewayServiceCleanup, GatewayServiceCleanupDriver, GatewayServiceCleanupDriverError,
    GatewayServiceIdentity, GatewayServiceInstanceLease, GatewayServiceInstanceState, time,
};

impl GatewayServiceCleanupDriver {
    /// Confirms a physically complete cleanup that may already be durable.
    ///
    /// This read-only check lets a retry resolve an acknowledged `Cleaned`
    /// response without requiring a renewed lease or repeating physical work.
    /// `Ok(false)` means the exact row still needs the ordinary retry path.
    ///
    /// # Errors
    ///
    /// Returns a redacted error for an unavailable lookup or a newer owner or
    /// fence.  The caller may retry an unavailable lookup without mutation.
    pub async fn confirm_cleaned_state(
        &self,
        cleanup: &GatewayServiceCleanup,
        lease: &GatewayServiceInstanceLease,
    ) -> Result<bool, GatewayServiceCleanupDriverError> {
        if !cleanup.vm_teardown_confirmed() || !cleanup.materializer_cleanup_confirmed() {
            return Ok(false);
        }
        match self.lookup_instance(cleanup.identity()).await? {
            Some(current) if current.state == GatewayServiceInstanceState::Cleaned => {
                exact_claim(&current, lease)
                    .then_some(true)
                    .ok_or(GatewayServiceCleanupDriverError::Stale)
            }
            Some(current) if exact_claim(&current, lease) => Ok(false),
            Some(_) => Err(GatewayServiceCleanupDriverError::Stale),
            None => Err(GatewayServiceCleanupDriverError::Unavailable),
        }
    }

    pub(super) async fn lookup_instance(
        &self,
        identity: GatewayServiceIdentity,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayServiceCleanupDriverError> {
        time::timeout(
            self.policy.database_timeout,
            self.targets.get_service_instance(identity),
        )
        .await
        .map_err(|_| GatewayServiceCleanupDriverError::Unavailable)?
        .map_err(|_| GatewayServiceCleanupDriverError::Unavailable)
    }

    pub(super) async fn confirm_cleaned(
        &self,
        identity: GatewayServiceIdentity,
        expected: &GatewayServiceInstanceLease,
    ) -> Result<bool, GatewayServiceCleanupDriverError> {
        match self.lookup_instance(identity).await? {
            Some(instance)
                if instance.identity == identity
                    && instance.state == GatewayServiceInstanceState::Cleaned =>
            {
                if exact_claim(&instance, expected) {
                    Ok(true)
                } else {
                    Err(GatewayServiceCleanupDriverError::Stale)
                }
            }
            Some(instance)
                if instance.identity == identity
                    && instance.owner_host_id == expected.owner_host_id
                    && instance.owner_uuid == expected.owner_uuid
                    && instance.fencing_token == expected.fencing_token =>
            {
                Err(GatewayServiceCleanupDriverError::Unavailable)
            }
            Some(instance) if instance.identity == identity => {
                Err(GatewayServiceCleanupDriverError::Stale)
            }
            Some(_) => Err(GatewayServiceCleanupDriverError::Stale),
            None => Err(GatewayServiceCleanupDriverError::Unavailable),
        }
    }
}
