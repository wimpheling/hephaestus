use super::claim_resolution::{same_claim, valid_claim_identity};
use super::{
    Arc, CleanupCompletion, CleanupRetryState, GatewayServiceCleanupDriver,
    GatewayServiceCleanupDriverError, GatewayServiceCleanupDriverOutcome,
    GatewayServiceCleanupDriverPolicy, GatewayServiceExpiredClaimRecovery,
    GatewayServiceInstanceLease, GatewayServiceInstanceState, GatewayServiceOwner,
    GatewayServiceOwnershipError, GatewayServiceSupervisorContext, Instant, Uuid, time,
};
pub(super) async fn run_cleanup_retry(
    id: Uuid,
    mut state: CleanupRetryState,
    renew_deadline: Instant,
    context: Arc<GatewayServiceSupervisorContext>,
) -> CleanupCompletion {
    let policy = GatewayServiceCleanupDriverPolicy {
        lease: context.policy.lease,
        database_timeout: context.policy.instance.probe_timeout,
    };
    let result = match GatewayServiceCleanupDriver::new(
        Arc::clone(&context.ownership),
        Arc::clone(&context.failure_store),
        Arc::clone(&context.targets),
        Arc::clone(&context.provider),
        Arc::clone(&context.resolver),
        context.owner.clone(),
        policy,
    ) {
        Ok(driver) => match driver
            .confirm_cleaned_state(&state.cleanup, &state.lease)
            .await
        {
            Ok(true) => Ok(()),
            Ok(false) | Err(GatewayServiceCleanupDriverError::Unavailable) => {
                match driver
                    .renew_and_prepare_stopping(&mut state.lease, renew_deadline)
                    .await
                {
                    Ok(cleanup_deadline) => {
                        match driver
                            .attempt(
                                &mut state.cleanup,
                                &mut state.lease,
                                &mut state.pending_failure,
                                cleanup_deadline,
                            )
                            .await
                        {
                            Ok(GatewayServiceCleanupDriverOutcome::Cleaned) => Ok(()),
                            Ok(GatewayServiceCleanupDriverOutcome::Pending { .. }) => {
                                Err(GatewayServiceCleanupDriverError::Unavailable)
                            }
                            Err(error) => Err(error),
                        }
                    }
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };
    CleanupCompletion { id, state, result }
}

pub(super) async fn run_cleanup_retry_with_recovery(
    id: Uuid,
    mut state: CleanupRetryState,
    deadline: Instant,
    context: Arc<GatewayServiceSupervisorContext>,
    recovery: Arc<dyn GatewayServiceExpiredClaimRecovery>,
) -> CleanupCompletion {
    let policy = GatewayServiceCleanupDriverPolicy {
        lease: context.policy.lease,
        database_timeout: context.policy.instance.probe_timeout,
    };
    let result = match GatewayServiceCleanupDriver::new(
        Arc::clone(&context.ownership),
        Arc::clone(&context.failure_store),
        Arc::clone(&context.targets),
        Arc::clone(&context.provider),
        Arc::clone(&context.resolver),
        context.owner.clone(),
        policy,
    ) {
        Ok(driver) => {
            let confirmation = driver
                .confirm_cleaned_state(&state.cleanup, &state.lease)
                .await;
            match confirmation {
                Ok(true) => Ok(()),
                Ok(false) | Err(GatewayServiceCleanupDriverError::Unavailable) => {
                    renew_or_recover_cleanup(&driver, &mut state, deadline, &context, &recovery)
                        .await
                }
                Err(GatewayServiceCleanupDriverError::Stale) => {
                    recover_and_cleanup(&driver, &mut state, deadline, &context, &recovery).await
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    };
    CleanupCompletion { id, state, result }
}

pub(super) async fn renew_or_recover_cleanup(
    driver: &GatewayServiceCleanupDriver,
    state: &mut CleanupRetryState,
    deadline: Instant,
    context: &GatewayServiceSupervisorContext,
    recovery: &Arc<dyn GatewayServiceExpiredClaimRecovery>,
) -> Result<(), GatewayServiceCleanupDriverError> {
    match driver
        .renew_and_prepare_stopping(&mut state.lease, deadline)
        .await
    {
        Ok(cleanup_deadline) => run_cleanup_attempt(driver, state, cleanup_deadline).await,
        Err(GatewayServiceCleanupDriverError::Stale) => {
            recover_and_cleanup(driver, state, deadline, context, recovery).await
        }
        Err(error) => Err(error),
    }
}

pub(super) async fn recover_and_cleanup(
    driver: &GatewayServiceCleanupDriver,
    state: &mut CleanupRetryState,
    deadline: Instant,
    context: &GatewayServiceSupervisorContext,
    recovery: &Arc<dyn GatewayServiceExpiredClaimRecovery>,
) -> Result<(), GatewayServiceCleanupDriverError> {
    let (candidate, recovery_started) = recover_expired_claim(
        recovery,
        &state.lease,
        &context.owner,
        context.policy.lease.lease_duration,
        context.policy.instance.probe_timeout,
        deadline,
        state.cleanup.vm_teardown_confirmed() && state.cleanup.materializer_cleanup_confirmed(),
    )
    .await?;
    // Adopt only after all identity, owner, VM, fence, and state checks pass.
    // A validated successor remains caller-owned even when its renewal fails.
    state.lease = candidate;
    if state.lease.state == GatewayServiceInstanceState::Cleaned {
        if state.cleanup.vm_teardown_confirmed()
            && state.cleanup.materializer_cleanup_confirmed()
            && driver
                .confirm_cleaned_state(&state.cleanup, &state.lease)
                .await?
        {
            return Ok(());
        }
        return Err(GatewayServiceCleanupDriverError::Unavailable);
    }
    let renewal_deadline = recovery_started
        .checked_add(context.policy.lease.lease_duration)
        .map_or(deadline, |candidate| candidate.min(deadline));
    // The observed successor grants no physical authority until this exact
    // lease is renewed under a fresh conservative deadline.
    let cleanup_deadline = driver
        .renew_and_prepare_stopping(&mut state.lease, renewal_deadline)
        .await?;
    run_cleanup_attempt(driver, state, cleanup_deadline).await
}

pub(super) async fn run_cleanup_attempt(
    driver: &GatewayServiceCleanupDriver,
    state: &mut CleanupRetryState,
    deadline: Instant,
) -> Result<(), GatewayServiceCleanupDriverError> {
    match driver
        .attempt(
            &mut state.cleanup,
            &mut state.lease,
            &mut state.pending_failure,
            deadline,
        )
        .await?
    {
        GatewayServiceCleanupDriverOutcome::Cleaned => Ok(()),
        GatewayServiceCleanupDriverOutcome::Pending { .. } => {
            Err(GatewayServiceCleanupDriverError::Unavailable)
        }
    }
}

pub(super) async fn recover_expired_claim(
    recovery: &Arc<dyn GatewayServiceExpiredClaimRecovery>,
    previous: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
    lease_duration: std::time::Duration,
    database_timeout: std::time::Duration,
    overall_deadline: Instant,
    physical_confirmed: bool,
) -> Result<(GatewayServiceInstanceLease, Instant), GatewayServiceCleanupDriverError> {
    if !valid_claim_identity(previous) {
        return Err(GatewayServiceCleanupDriverError::Stale);
    }
    if overall_deadline <= Instant::now() {
        return Err(GatewayServiceCleanupDriverError::Stale);
    }
    let call_started = Instant::now();
    let database_deadline = call_started
        .checked_add(database_timeout)
        .ok_or(GatewayServiceCleanupDriverError::InvalidInput)?;
    let call_deadline = database_deadline.min(overall_deadline);
    let takeover = time::timeout_at(
        call_deadline,
        recovery.claim_expired_instance(previous, owner, lease_duration),
    )
    .await;
    let candidate = match takeover {
        Ok(Ok(candidate)) => candidate,
        Ok(Err(
            GatewayServiceOwnershipError::StaleLease | GatewayServiceOwnershipError::Unavailable,
        ))
        | Err(_) => {
            let resolve_started = Instant::now();
            let resolve_deadline = resolve_started
                .checked_add(database_timeout)
                .ok_or(GatewayServiceCleanupDriverError::InvalidInput)?
                .min(overall_deadline);
            time::timeout_at(
                resolve_deadline,
                recovery.resolve_exact_instance(previous.identity),
            )
            .await
            .map_err(|_| GatewayServiceCleanupDriverError::Unavailable)?
            .map_err(map_recovery_ownership_error)?
            .ok_or(GatewayServiceCleanupDriverError::Unavailable)?
        }
        Ok(Err(error)) => return Err(map_recovery_ownership_error(error)),
    };
    let successor_valid =
        valid_expired_recovery_successor(previous, &candidate, owner, physical_confirmed);
    let cleaned_valid = valid_claim_identity(previous)
        && valid_claim_identity(&candidate)
        && physical_confirmed
        && candidate.state == GatewayServiceInstanceState::Cleaned
        && same_claim(&candidate, previous);
    if !(successor_valid || cleaned_valid) {
        return Err(GatewayServiceCleanupDriverError::Stale);
    }
    Ok((candidate, call_started))
}

pub(super) fn valid_expired_recovery_successor(
    previous: &GatewayServiceInstanceLease,
    candidate: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
    physical_confirmed: bool,
) -> bool {
    let same_host =
        candidate.owner_host_id == owner.host_id && previous.owner_host_id == owner.host_id;
    previous
        .fencing_token
        .checked_add(1)
        .is_some_and(|next_fence| {
            valid_claim_identity(previous)
                && valid_claim_identity(candidate)
                && candidate.identity == previous.identity
                && candidate.vm_id == previous.vm_id
                && same_host
                && candidate.owner_uuid == owner.owner_uuid
                && candidate.fencing_token == next_fence
                && (candidate.state == GatewayServiceInstanceState::Stopping
                    || (physical_confirmed
                        && candidate.state == GatewayServiceInstanceState::Cleaned))
                && candidate.lease_expires_at > candidate.heartbeat_at
        })
}

const fn map_recovery_ownership_error(
    error: GatewayServiceOwnershipError,
) -> GatewayServiceCleanupDriverError {
    match error {
        GatewayServiceOwnershipError::StaleLease => GatewayServiceCleanupDriverError::Stale,
        GatewayServiceOwnershipError::Unavailable => GatewayServiceCleanupDriverError::Unavailable,
        GatewayServiceOwnershipError::Conflict | GatewayServiceOwnershipError::InvalidArgument => {
            GatewayServiceCleanupDriverError::Rejected
        }
    }
}
