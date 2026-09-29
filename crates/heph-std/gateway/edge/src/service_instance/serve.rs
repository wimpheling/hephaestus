use super::helpers::{FailureOutcome, map_exit};
use super::{
    Command, ServiceInstance, ServiceInstanceError, ServiceProbePolicy, ServiceWorkerState,
    StopMode, probe_private_service_http, time,
};

impl ServiceInstance {
    pub(super) async fn serve_ready(&mut self) -> Result<(), ServiceInstanceError> {
        let mut wait = Box::pin(self.vm.wait());
        loop {
            tokio::select! {
                () = self.cancellation.cancelled() => return self.finish_with_failure(FailureOutcome::shutdown()).await,
                exit = &mut wait => return self.finish_with_failure(map_exit(&exit)).await,
                command = self.commands.recv() => match command {
                    Some(Command::Health(reply)) => {
                        let probe = self.health_probe();
                        tokio::pin!(probe);
                        tokio::select! {
                            () = self.cancellation.cancelled() => return self.finish_with_failure(FailureOutcome::shutdown()).await,
                            exit = &mut wait => return self.finish_with_failure(map_exit(&exit)).await,
                            result = &mut probe => { let _ = reply.send(result); }
                        }
                    }
                    None => return self.finish_with_failure(FailureOutcome::shutdown()).await,
                },
            }
        }
    }

    async fn health_probe(&self) -> Result<http::StatusCode, ServiceInstanceError> {
        probe_private_service_http(
            self.vm.as_ref(),
            &self.launch.service.health_path,
            &self.authority,
            ServiceProbePolicy::new(self.policy.probe_timeout),
        )
        .await
        .map(|success| success.status)
        .map_err(ServiceInstanceError::HealthProbe)
    }

    pub(super) async fn finish_with_failure(
        &self,
        outcome: FailureOutcome,
    ) -> Result<(), ServiceInstanceError> {
        if let Some(failure) = outcome.report {
            *self.failure.write().expect("service failure snapshot lock") = Some(failure);
        }
        let primary = outcome.error;
        self.set_state(ServiceWorkerState::Stopping);
        let stop = time::timeout(
            self.policy.shutdown_timeout,
            self.vm.stop(StopMode::Graceful {
                timeout: self.policy.shutdown_timeout,
            }),
        )
        .await;
        let _ = stop;
        let destroyed = time::timeout(self.policy.shutdown_timeout, self.vm.destroy()).await;
        if destroyed.is_err() || destroyed.ok().and_then(Result::err).is_some() {
            self.set_state(ServiceWorkerState::CleanupIncomplete);
            return Err(ServiceInstanceError::CleanupIncomplete);
        }
        let cleaned = time::timeout(
            self.policy.shutdown_timeout,
            self.resolver.cleanup_service_launch(self.launch.identity),
        )
        .await;
        if cleaned.is_err() || cleaned.ok().and_then(Result::err).is_some() {
            self.set_state(ServiceWorkerState::CleanupIncomplete);
            return Err(ServiceInstanceError::CleanupIncomplete);
        }
        self.set_state(ServiceWorkerState::Stopped);
        if matches!(primary, ServiceInstanceError::Shutdown) {
            Ok(())
        } else {
            Err(primary)
        }
    }
}
