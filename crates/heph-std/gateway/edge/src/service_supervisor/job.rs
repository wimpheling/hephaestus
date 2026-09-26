use super::{
    Arc, CancellationToken, GatewayServiceCapacity, GatewayServiceCapacityToken,
    GatewayServiceCoordinator, GatewayServiceCoordinatorFailure,
    GatewayServiceCoordinatorFailureReason, GatewayServiceCoordinatorStatus,
    GatewayServiceInstanceLease, GatewayServiceLogWriterConfig, GatewayServiceOwnershipError,
    GatewayServiceStartupRequest, GatewayServiceSupervisorContext,
    GatewayServiceSupervisorJobStatus, Instant, JobCompletion, JobTerminal, Mutex,
    StartupDeadlines, Uuid, time, watch,
};
// This function intentionally owns the complete claim/coordinator state
// machine so cancellation cannot drop a durable claim outside the supervisor.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub(super) async fn run_job(
    id: Uuid,
    request: GatewayServiceStartupRequest,
    token: GatewayServiceCapacityToken,
    deadlines: StartupDeadlines,
    context: Arc<GatewayServiceSupervisorContext>,
    log_writer: Option<GatewayServiceLogWriterConfig>,
    capacity: Arc<Mutex<GatewayServiceCapacity>>,
    cancellation: CancellationToken,
    status: watch::Sender<GatewayServiceSupervisorJobStatus>,
    mut drain: watch::Receiver<bool>,
) -> JobCompletion {
    if cancellation.is_cancelled() || Instant::now() >= deadlines.startup_deadline {
        let status_value = if cancellation.is_cancelled() {
            GatewayServiceSupervisorJobStatus::Cancelled
        } else {
            GatewayServiceSupervisorJobStatus::Failed
        };
        return terminal(
            id,
            token,
            &capacity,
            &status,
            status_value,
            None,
            false,
            true,
            None,
        );
    }
    let claim = context.ownership.claim_new(
        request.gateway_id,
        request.revision_id,
        &context.owner,
        context.policy.lease.lease_duration,
    );
    tokio::pin!(claim);
    let claim_result = tokio::select! {
        result = &mut claim => result,
        () = cancellation.cancelled() => {
            let _ = status.send(GatewayServiceSupervisorJobStatus::Cancelled);
            claim.await
        }
        () = time::sleep_until(deadlines.startup_deadline) => {
            let _ = status.send(GatewayServiceSupervisorJobStatus::Cancelled);
            claim.await
        }
    };
    let lease = match claim_result {
        Ok(lease) => lease,
        Err(
            GatewayServiceOwnershipError::Conflict
            | GatewayServiceOwnershipError::InvalidArgument
            | GatewayServiceOwnershipError::StaleLease,
        ) => {
            return terminal(
                id,
                token,
                &capacity,
                &status,
                GatewayServiceSupervisorJobStatus::Failed,
                None,
                false,
                true,
                None,
            );
        }
        Err(GatewayServiceOwnershipError::Unavailable) => {
            return terminal(
                id,
                token,
                &capacity,
                &status,
                GatewayServiceSupervisorJobStatus::Uncertain,
                None,
                false,
                false,
                None,
            );
        }
    };
    if lease.identity.gateway_id != request.gateway_id
        || lease.identity.revision_id != request.revision_id
    {
        return terminal(
            id,
            token,
            &capacity,
            &status,
            GatewayServiceSupervisorJobStatus::Failed,
            Some(lease),
            false,
            false,
            None,
        );
    }
    if cancellation.is_cancelled() || Instant::now() >= deadlines.startup_deadline {
        let mut completion = terminal(
            id,
            token,
            &capacity,
            &status,
            GatewayServiceSupervisorJobStatus::Cancelled,
            Some(lease),
            false,
            false,
            None,
        );
        completion.terminal.claim_cleanup_reason = Some(if cancellation.is_cancelled() {
            GatewayServiceCoordinatorFailureReason::Cancelled
        } else {
            GatewayServiceCoordinatorFailureReason::StartupDeadline
        });
        return completion;
    }
    let _ = status.send(GatewayServiceSupervisorJobStatus::Starting);
    let coordinator = GatewayServiceCoordinator::new(
        lease.clone(),
        context.owner.clone(),
        deadlines.initial_lease_deadline,
        deadlines.startup_deadline,
        request.intent,
        Arc::clone(&context.ownership),
        Arc::clone(&context.failure_store),
        Arc::clone(&context.resolver),
        Arc::clone(&context.provider),
        Arc::clone(&context.targets),
        context.registry.clone(),
        context.service_authority.clone(),
        context.policy,
    );
    let Ok((coordinator, control)) = coordinator else {
        return terminal(
            id,
            token,
            &capacity,
            &status,
            GatewayServiceSupervisorJobStatus::Failed,
            Some(lease),
            false,
            false,
            None,
        );
    };
    let coordinator_status = control.subscribe();
    let mut coordinator_status = coordinator_status;
    let mut run = Box::pin(
        (if let Some(config) = log_writer {
            coordinator.with_log_writer(config)
        } else {
            coordinator
        })
        .run(),
    );
    let mut cancel_sent = false;
    let mut startup_finished = false;
    let mut status_open = true;
    let mut drain_open = true;
    if *drain.borrow() {
        control.request_drain();
    }
    let result = loop {
        tokio::select! {
            result = &mut run => break result,
            changed = coordinator_status.changed(), if status_open => {
                if changed.is_err() {
                    status_open = false;
                    continue;
                }
                let current = *coordinator_status.borrow();
                if current == GatewayServiceCoordinatorStatus::Ready && !startup_finished {
                    startup_finished = finish_startup(&capacity, token);
                    let _ = status.send(GatewayServiceSupervisorJobStatus::Ready);
                } else if current == GatewayServiceCoordinatorStatus::Starting || current == GatewayServiceCoordinatorStatus::Preparing || current == GatewayServiceCoordinatorStatus::Probing {
                    let _ = status.send(GatewayServiceSupervisorJobStatus::Starting);
                }
            }
            () = cancellation.cancelled(), if !cancel_sent => {
                cancel_sent = true;
                control.cancel();
            }
            changed = drain.changed(), if drain_open => {
                match changed {
                    Ok(()) if *drain.borrow() => control.request_drain(),
                    Ok(()) => {}
                    Err(_) => drain_open = false,
                }
            }
        }
    };
    if !startup_finished {
        startup_finished = finish_startup(&capacity, token);
    }
    match result {
        Ok(()) => terminal(
            id,
            token,
            &capacity,
            &status,
            GatewayServiceSupervisorJobStatus::Settled,
            None,
            startup_finished,
            true,
            None,
        ),
        Err(failure) => {
            let released = failure.physical_cleanup_complete && failure.durable_cleanup_complete;
            let status_value = if cancellation.is_cancelled() {
                GatewayServiceSupervisorJobStatus::Cancelled
            } else {
                GatewayServiceSupervisorJobStatus::Failed
            };
            terminal(
                id,
                token,
                &capacity,
                &status,
                status_value,
                Some(failure.lease.clone()),
                startup_finished,
                released,
                Some(failure),
            )
        }
    }
}

pub(super) fn finish_startup(
    capacity: &Arc<Mutex<GatewayServiceCapacity>>,
    token: GatewayServiceCapacityToken,
) -> bool {
    capacity
        .lock()
        .expect("service capacity lock")
        .finish_startup(token)
        .is_ok()
}

pub(super) fn complete_capacity(
    capacity: &Arc<Mutex<GatewayServiceCapacity>>,
    token: GatewayServiceCapacityToken,
) -> bool {
    capacity
        .lock()
        .expect("service capacity lock")
        .complete(token)
        .is_ok()
}

// The terminal record keeps every ownership-bearing value together. Its
// argument count is deliberate: callers must state cleanup completion before
// releasing capacity.
#[allow(clippy::too_many_arguments)]
pub(super) fn terminal(
    id: Uuid,
    token: GatewayServiceCapacityToken,
    capacity: &Arc<Mutex<GatewayServiceCapacity>>,
    status: &watch::Sender<GatewayServiceSupervisorJobStatus>,
    status_value: GatewayServiceSupervisorJobStatus,
    lease: Option<GatewayServiceInstanceLease>,
    startup_finished: bool,
    release_capacity: bool,
    coordinator_failure: Option<GatewayServiceCoordinatorFailure>,
) -> JobCompletion {
    if !startup_finished {
        let _ = finish_startup(capacity, token);
    }
    let capacity_released = release_capacity && complete_capacity(capacity, token);
    let _ = status.send(status_value);
    JobCompletion {
        id,
        status: status_value,
        capacity_released,
        terminal: JobTerminal {
            lease,
            claim_uncertain: status_value == GatewayServiceSupervisorJobStatus::Uncertain,
            coordinator_failure,
            claim_cleanup_reason: None,
        },
    }
}
