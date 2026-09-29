use super::helpers::{FailureOutcome, drain_events, failure_for_startup, map_exit, observe_event};
use super::{
    Instant, ServiceDiagnostics, ServiceInstance, ServiceInstanceError, ServiceProbeError,
    ServiceProbePolicy, ServiceWorkerState, VmError, probe_private_service_http, time,
};

impl ServiceInstance {
    /// Runs the VM lifecycle until shutdown, exit, or cleanup failure.
    ///
    /// # Errors
    ///
    /// Returns a redacted lifecycle or cleanup error. A successful return means
    /// that provider and materializer cleanup both completed.
    pub async fn run(mut self) -> Result<(), ServiceInstanceError> {
        let mut events = self.vm.subscribe_events();
        let mut diagnostics = ServiceDiagnostics::new(self.diagnostics.clone());
        let logs = self.logs.clone();
        let mut events_open = true;
        let mut lifecycle = Box::pin(self.run_lifecycle());
        loop {
            tokio::select! {
                biased;
                result = &mut lifecycle => {
                    drop(lifecycle);
                    drain_events(&mut events, &mut diagnostics, &mut events_open, logs.as_ref());
                    return result;
                }
                event = events.recv(), if events_open => {
                    events_open = observe_event(&mut diagnostics, logs.as_ref(), event);
                }
            }
        }
    }

    async fn run_lifecycle(&mut self) -> Result<(), ServiceInstanceError> {
        let startup_deadline = Instant::now()
            .checked_add(self.policy.startup_timeout)
            .ok_or(ServiceInstanceError::InvalidPolicy)?;
        self.set_state(ServiceWorkerState::Starting);
        match time::timeout_at(startup_deadline, self.start_vm()).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                let reason = if self.cancellation.is_cancelled() {
                    ServiceInstanceError::Shutdown
                } else {
                    ServiceInstanceError::StartupFailed
                };
                let failure = failure_for_startup(&reason);
                return self
                    .finish_with_failure(FailureOutcome {
                        error: reason,
                        report: failure,
                    })
                    .await;
            }
            Err(_) => {
                return self
                    .finish_with_failure(FailureOutcome::startup_timeout())
                    .await;
            }
        }
        self.set_state(ServiceWorkerState::Probing);
        match self.await_readiness(startup_deadline).await {
            Ok(()) => self.set_state(ServiceWorkerState::Ready),
            Err(outcome) => return self.finish_with_failure(outcome).await,
        }
        self.serve_ready().await
    }

    pub(super) fn set_state(&self, state: ServiceWorkerState) {
        let _ = self.states.send(state);
        self.diagnostics.send_modify(|snapshot| {
            snapshot.worker_state = state;
        });
        tracing::debug!(
            gateway_id = %self.launch.identity.gateway_id,
            revision_id = %self.launch.identity.revision_id,
            instance_id = %self.launch.identity.instance_id,
            state = ?state,
            "gateway service lifecycle state changed"
        );
    }

    async fn start_vm(&self) -> Result<(), VmError> {
        tokio::select! {
            () = self.cancellation.cancelled() => Err(VmError::InvalidState("service cancelled")),
            result = self.vm.start() => result,
        }
    }

    async fn await_readiness(&self, deadline: Instant) -> Result<(), FailureOutcome> {
        let mut wait = Box::pin(self.vm.wait());
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(FailureOutcome::readiness(
                    ServiceInstanceError::StartupTimeout,
                ));
            }
            let probe = time::timeout_at(
                deadline,
                probe_private_service_http(
                    self.vm.as_ref(),
                    &self.launch.service.readiness_path,
                    &self.authority,
                    ServiceProbePolicy::new(self.policy.probe_timeout.min(remaining)),
                ),
            );
            tokio::pin!(probe);
            tokio::select! {
                () = self.cancellation.cancelled() => return Err(FailureOutcome::shutdown()),
                exit = &mut wait => return Err(map_exit(&exit)),
                result = &mut probe => match result {
                    Err(_) => return Err(FailureOutcome::readiness(ServiceInstanceError::StartupTimeout)),
                    Ok(Ok(_)) => return Ok(()),
                    Ok(Err(ServiceProbeError::InvalidPolicy | ServiceProbeError::InvalidAuthority | ServiceProbeError::Contract)) => return Err(FailureOutcome::readiness(ServiceInstanceError::StartupFailed)),
                    Ok(Err(_)) => {}
                },
            }
            let pause = self
                .policy
                .probe_interval
                .min(deadline.saturating_duration_since(Instant::now()));
            if pause.is_zero() {
                return Err(FailureOutcome::readiness(
                    ServiceInstanceError::StartupTimeout,
                ));
            }
            tokio::select! {
                () = self.cancellation.cancelled() => return Err(FailureOutcome::shutdown()),
                exit = &mut wait => return Err(map_exit(&exit)),
                () = time::sleep(pause) => {}
            }
        }
    }
}
