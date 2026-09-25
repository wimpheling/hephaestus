use super::monitor::{MonitorState, exact_claim, validate_stopping};
use super::{
    Arc, GatewayServiceCleanup, GatewayServiceCleanupDriver, GatewayServiceCleanupDriverError,
    GatewayServiceCleanupDriverOutcome, GatewayServiceFailure, GatewayServiceFailureCode,
    GatewayServiceFailureStoreError, GatewayServiceInstanceLease, GatewayServiceInstanceState,
    GatewayServiceLeaseMonitor, GatewayServiceOwnershipError, Instant,
};

impl GatewayServiceCleanupDriver {
    /// Performs one bounded cleanup attempt for an already-owned stopping row.
    ///
    /// The caller retains `cleanup`, `lease`, and `pending_failure` across every
    /// return.  A lease loss never abandons the physical future, and no task is
    /// detached from this call.
    ///
    /// # Errors
    ///
    /// Returns a redacted durable-boundary error while preserving all caller
    /// owned cleanup and failure state for a later attempt.
    #[allow(clippy::too_many_lines)] // One ordered state machine protects cleanup/failure/lease invariants.
    pub async fn attempt(
        &self,
        cleanup: &mut GatewayServiceCleanup,
        lease: &mut GatewayServiceInstanceLease,
        pending_failure: &mut Option<GatewayServiceFailure>,
        initial_deadline: Instant,
    ) -> Result<GatewayServiceCleanupDriverOutcome, GatewayServiceCleanupDriverError> {
        validate_stopping(lease, &self.owner, cleanup.identity())?;
        let identity = cleanup.identity();
        if cleanup.vm_teardown_confirmed()
            && cleanup.materializer_cleanup_confirmed()
            && initial_deadline <= Instant::now()
        {
            match self.lookup_instance(identity).await? {
                Some(current) if current.state == GatewayServiceInstanceState::Cleaned => {
                    if exact_claim(&current, lease) {
                        return Ok(GatewayServiceCleanupDriverOutcome::Cleaned);
                    }
                    return Err(GatewayServiceCleanupDriverError::Stale);
                }
                Some(_) => return Err(GatewayServiceCleanupDriverError::Stale),
                None => return Err(GatewayServiceCleanupDriverError::Unavailable),
            }
        }
        let (monitor, control) = GatewayServiceLeaseMonitor::new(
            Arc::clone(&self.ownership),
            lease.clone(),
            self.owner.clone(),
            self.policy.lease,
            initial_deadline,
        )
        .map_err(|_| GatewayServiceCleanupDriverError::InvalidInput)?;
        let status = control.subscribe();
        let mut monitor_state = MonitorState {
            future: Box::pin(monitor.run()),
            control,
            status,
            database_timeout: self.policy.database_timeout,
            done: false,
            lost: None,
        };

        let current = match monitor_state
            .await_durable(self.targets.get_service_instance(identity), lease)
            .await
        {
            Ok(Ok(current)) => current,
            Ok(Err(_)) | Err(GatewayServiceCleanupDriverError::Unavailable) => {
                monitor_state.stop().await;
                return Err(GatewayServiceCleanupDriverError::Unavailable);
            }
            Err(error) => {
                monitor_state.stop().await;
                return Err(error);
            }
        };
        match current {
            Some(current)
                if exact_claim(&current, lease)
                    && current.state == GatewayServiceInstanceState::Stopping => {}
            Some(current) if current.state == GatewayServiceInstanceState::Cleaned => {
                monitor_state.stop().await;
                if cleanup.vm_teardown_confirmed()
                    && cleanup.materializer_cleanup_confirmed()
                    && exact_claim(&current, lease)
                {
                    return Ok(GatewayServiceCleanupDriverOutcome::Cleaned);
                }
                return Err(GatewayServiceCleanupDriverError::Stale);
            }
            Some(_) => {
                monitor_state.stop().await;
                return Err(GatewayServiceCleanupDriverError::Stale);
            }
            None => {
                monitor_state.stop().await;
                return Err(GatewayServiceCleanupDriverError::Unavailable);
            }
        }

        let mut physical =
            Box::pin(cleanup.attempt(self.provider.as_ref(), self.resolver.as_ref()));
        let physical_result = monitor_state
            .await_operation(&mut physical, lease, true)
            .await
            .ok_or(GatewayServiceCleanupDriverError::Unavailable)?;
        drop(physical);
        let physical_complete = physical_result.is_ok();
        if !physical_complete && pending_failure.is_none() {
            *pending_failure = Some(
                GatewayServiceFailure::new(GatewayServiceFailureCode::Cleanup, None, None)
                    .map_err(|_| GatewayServiceCleanupDriverError::Rejected)?,
            );
        }
        if let Some(reason) = monitor_state.lost {
            monitor_state.stop().await;
            let _ = reason;
            return Ok(GatewayServiceCleanupDriverOutcome::Pending {
                physical_complete,
                failure_recorded: false,
                lease_lost: true,
            });
        }

        if let Some(failure) = *pending_failure {
            let operation_lease = lease.clone();
            let operation =
                self.failure_store
                    .record_failure(&operation_lease, &self.owner, failure);
            match monitor_state.await_durable(operation, lease).await {
                Ok(Ok(())) => *pending_failure = None,
                Ok(Err(GatewayServiceFailureStoreError::StaleLease))
                | Err(GatewayServiceCleanupDriverError::Stale) => {
                    monitor_state.stop().await;
                    return Err(GatewayServiceCleanupDriverError::Stale);
                }
                Ok(Err(GatewayServiceFailureStoreError::Unavailable))
                | Err(GatewayServiceCleanupDriverError::Unavailable) => {
                    monitor_state.stop().await;
                    return Err(GatewayServiceCleanupDriverError::Unavailable);
                }
                Ok(Err(GatewayServiceFailureStoreError::InvalidArgument))
                | Err(
                    GatewayServiceCleanupDriverError::Rejected
                    | GatewayServiceCleanupDriverError::InvalidInput,
                ) => {
                    monitor_state.stop().await;
                    return Err(GatewayServiceCleanupDriverError::Rejected);
                }
            }
        }
        if !physical_complete {
            let lease_lost = monitor_state.lost.is_some();
            monitor_state.stop().await;
            return Ok(GatewayServiceCleanupDriverOutcome::Pending {
                physical_complete: false,
                failure_recorded: pending_failure.is_none(),
                lease_lost,
            });
        }
        if monitor_state.lost.is_some() {
            monitor_state.stop().await;
            return Ok(GatewayServiceCleanupDriverOutcome::Pending {
                physical_complete: true,
                failure_recorded: pending_failure.is_none(),
                lease_lost: true,
            });
        }

        let operation_lease = lease.clone();
        let operation = self.ownership.mark_cleaned(&operation_lease, &self.owner);
        match monitor_state.await_durable(operation, lease).await {
            Ok(Ok(())) => {
                monitor_state.stop().await;
                Ok(GatewayServiceCleanupDriverOutcome::Cleaned)
            }
            Ok(Err(
                GatewayServiceOwnershipError::StaleLease
                | GatewayServiceOwnershipError::Unavailable,
            ))
            | Err(
                GatewayServiceCleanupDriverError::Stale
                | GatewayServiceCleanupDriverError::Unavailable,
            ) => {
                monitor_state.stop().await;
                if self.confirm_cleaned(identity, lease).await? {
                    Ok(GatewayServiceCleanupDriverOutcome::Cleaned)
                } else {
                    Err(GatewayServiceCleanupDriverError::Unavailable)
                }
            }
            Ok(Err(
                GatewayServiceOwnershipError::InvalidArgument
                | GatewayServiceOwnershipError::Conflict,
            ))
            | Err(
                GatewayServiceCleanupDriverError::Rejected
                | GatewayServiceCleanupDriverError::InvalidInput,
            ) => {
                monitor_state.stop().await;
                Err(GatewayServiceCleanupDriverError::Rejected)
            }
        }
    }
}
