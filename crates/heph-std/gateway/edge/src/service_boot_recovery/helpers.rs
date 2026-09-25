use super::{
    BootCleanupState, CleanupCompletion, CleanupFuture, GatewayServiceBootRecoveryContext,
    GatewayServiceCleanupDriver, GatewayServiceCleanupDriverError,
    GatewayServiceCleanupDriverOutcome, GatewayServiceInstanceLease, GatewayServiceInstanceState,
    GatewayServiceOwner, GatewayServiceOwnershipError, Instant,
};
use std::{sync::Arc, task::Poll};

pub(super) async fn run_cleanup(
    context: Arc<GatewayServiceBootRecoveryContext>,
    mut state: BootCleanupState,
    deadline: Instant,
) -> CleanupCompletion {
    let result = match GatewayServiceCleanupDriver::new(
        Arc::clone(&context.ownership),
        Arc::clone(&context.failure_store),
        Arc::clone(&context.targets),
        Arc::clone(&context.provider),
        Arc::clone(&context.resolver),
        context.owner.clone(),
        context.cleanup_policy,
    ) {
        Ok(driver) => {
            driver
                .attempt(
                    &mut state.cleanup,
                    &mut state.lease,
                    &mut state.pending_failure,
                    deadline,
                )
                .await
        }
        Err(error) => Err(error),
    };
    CleanupCompletion { state, result }
}

pub(super) async fn run_retry(
    context: Arc<GatewayServiceBootRecoveryContext>,
    mut state: BootCleanupState,
) -> CleanupCompletion {
    let call_started = Instant::now();
    let deadline = call_started
        .checked_add(context.cleanup_policy.lease.lease_duration)
        .unwrap_or(call_started);
    let result = match cleanup_driver(&context) {
        Ok(driver)
            if state.cleanup.vm_teardown_confirmed()
                && state.cleanup.materializer_cleanup_confirmed() =>
        {
            match driver
                .confirm_cleaned_state(&state.cleanup, &state.lease)
                .await
            {
                Ok(true) => Ok(GatewayServiceCleanupDriverOutcome::Cleaned),
                Ok(false) => retry_takeover(&context, &mut state, deadline, driver).await,
                Err(error) => Err(error),
            }
        }
        Ok(driver) => retry_takeover(&context, &mut state, deadline, driver).await,
        Err(error) => Err(error),
    };
    CleanupCompletion { state, result }
}

async fn retry_takeover(
    context: &GatewayServiceBootRecoveryContext,
    state: &mut BootCleanupState,
    deadline: Instant,
    driver: GatewayServiceCleanupDriver,
) -> Result<GatewayServiceCleanupDriverOutcome, GatewayServiceCleanupDriverError> {
    let old_lease = state.lease.clone();
    let takeover = context.exact_recovery.claim_expired_instance(
        &old_lease,
        &context.owner,
        context.cleanup_policy.lease.lease_duration,
    );
    match takeover.await {
        Ok(lease) if valid_exact_takeover_lease(&lease, &context.owner, &old_lease) => {
            state.lease = lease;
            driver
                .attempt(
                    &mut state.cleanup,
                    &mut state.lease,
                    &mut state.pending_failure,
                    deadline,
                )
                .await
        }
        Ok(_) => Err(GatewayServiceCleanupDriverError::Stale),
        Err(GatewayServiceOwnershipError::StaleLease) => {
            Err(GatewayServiceCleanupDriverError::Stale)
        }
        Err(_) => Err(GatewayServiceCleanupDriverError::Unavailable),
    }
}

fn cleanup_driver(
    context: &GatewayServiceBootRecoveryContext,
) -> Result<GatewayServiceCleanupDriver, GatewayServiceCleanupDriverError> {
    GatewayServiceCleanupDriver::new(
        Arc::clone(&context.ownership),
        Arc::clone(&context.failure_store),
        Arc::clone(&context.targets),
        Arc::clone(&context.provider),
        Arc::clone(&context.resolver),
        context.owner.clone(),
        context.cleanup_policy,
    )
}

pub(super) async fn poll_cleanup_jobs(jobs: &mut Vec<CleanupFuture>) -> Option<CleanupCompletion> {
    std::future::poll_fn(|context| {
        for index in (0..jobs.len()).rev() {
            if let Poll::Ready(completion) = jobs[index].as_mut().poll(context) {
                drop(jobs.swap_remove(index));
                return Poll::Ready(Some(completion));
            }
        }
        if jobs.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Pending
        }
    })
    .await
}

pub(super) fn valid_recovery_lease(
    lease: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
) -> bool {
    let same_host = lease.owner_host_id == owner.host_id;
    !lease.identity.instance_id.is_nil()
        && !lease.identity.gateway_id.is_nil()
        && !lease.identity.revision_id.is_nil()
        && same_host
        && lease.owner_uuid == owner.owner_uuid
        && lease.fencing_token > 0
        && lease.vm_id == format!("gateway-service-{}", lease.identity.instance_id)
        && lease.state == GatewayServiceInstanceState::Stopping
        && lease.lease_expires_at > lease.heartbeat_at
}

pub(super) fn valid_owned_recovery_lease(
    lease: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
    expected: &GatewayServiceInstanceLease,
) -> bool {
    valid_recovery_lease(lease, owner)
        && lease.identity == expected.identity
        && lease.vm_id == expected.vm_id
        && lease.fencing_token == expected.fencing_token
}

pub(super) fn valid_exact_takeover_lease(
    lease: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
    previous: &GatewayServiceInstanceLease,
) -> bool {
    valid_recovery_lease(lease, owner)
        && lease.identity == previous.identity
        && lease.vm_id == previous.vm_id
        && lease.fencing_token > previous.fencing_token
}
