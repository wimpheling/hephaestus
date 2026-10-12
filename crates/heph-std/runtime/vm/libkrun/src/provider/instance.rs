use super::common::{
    Arc, BoxedPrivateServiceConnection, ErrorSnapshot, LibkrunInstance, Lifecycle, Ordering,
    PRIVATE_HTTP_TIMEOUT, PROVIDER_NAME, PrivateServiceConnectionMessage, StopMode, VmError,
    VmEvent, VmExit, VmId, VmInstance, WorkerCommand, async_trait, broadcast, info, oneshot,
    timeout,
};

use super::{
    helpers::{provider_error, unavailable_error},
    http::{private_http_request_message, wait_start_result},
};

#[async_trait]
impl VmInstance for LibkrunInstance {
    fn id(&self) -> &VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        // This marker's Drop must run after the operation lock is released.
        #[allow(clippy::collection_is_never_read)]
        let _activity;
        let _operation = self.operation.lock().await;
        let registry = self.registry.upgrade();
        _activity = if let Some(registry) = registry {
            Some(registry.admit_start(&self.provider_ids, self.birth).await?)
        } else {
            // Only isolated unit fixtures lack a provider-admitted birth.
            #[cfg(test)]
            if self.birth != 0 {
                return Err(VmError::Destroyed);
            }
            #[cfg(not(test))]
            return Err(VmError::Destroyed);
            #[cfg(test)]
            None
        };
        let mut result_rx = self.start_result.subscribe();
        let leader = {
            let mut state = self.state.lock().await;
            let leader = match &*state {
                Lifecycle::Provisioned => {
                    *state = Lifecycle::Starting;
                    true
                }
                Lifecycle::Starting => false,
                Lifecycle::Running | Lifecycle::Stopping => return Ok(()),
                Lifecycle::Exited => {
                    return Err(VmError::InvalidState("an exited VM cannot be restarted"));
                }
                Lifecycle::StartFailed(error) => return Err(error.to_vm_error()),
                Lifecycle::Destroying { .. } | Lifecycle::Destroyed => {
                    return Err(VmError::Destroyed);
                }
            };
            drop(state);
            leader
        };

        let mut destroying = self.destroy_requested.subscribe();
        let mut stopping = self.stop_requested.subscribe();
        if !leader {
            return tokio::select! {
                result = wait_start_result(&mut result_rx) => result,
                _requested = stopping.wait_for(|requested| *requested) => Err(VmError::InvalidState("startup interrupted by stop")),
                _requested = destroying.wait_for(|requested| *requested) => Err(VmError::Destroyed),
            };
        }

        info!(vm_id = %self.id.0, "starting libkrun worker");
        let mut outcome = tokio::select! {
            outcome = self.start_leader() => outcome,
            _requested = stopping.wait_for(|requested| *requested) => Err(VmError::InvalidState("startup interrupted by stop")),
            _requested = destroying.wait_for(|requested| *requested) => Err(VmError::Destroyed),
        };
        let interrupted_during_start = {
            let mut state = self.state.lock().await;
            if matches!(*state, Lifecycle::Destroying { .. } | Lifecycle::Destroyed) {
                outcome = Err(VmError::Destroyed);
                true
            } else if *self.stop_requested.borrow() {
                // Leave Starting for the serialized stop to issue termination.
                // A stop request is neither startup success nor cleanup proof.
                outcome = Err(VmError::InvalidState("startup interrupted by stop"));
                true
            } else {
                match &outcome {
                    Ok(()) => *state = Lifecycle::Running,
                    Err(error) => {
                        *state = Lifecycle::StartFailed(ErrorSnapshot::from_vm_error(error));
                    }
                }
                false
            }
        };
        let snapshot = outcome.as_ref().err().map(ErrorSnapshot::from_vm_error);
        self.start_result
            .send_replace(Some(snapshot.map_or(Ok(()), Err)));
        if outcome.is_err() && !interrupted_during_start {
            let _cleanup_result = self.force_cleanup_locked(true).await;
        }
        outcome
    }

    async fn stop(&self, mode: StopMode) -> Result<(), VmError> {
        {
            let state = self.state.lock().await;
            match &*state {
                Lifecycle::Provisioned
                | Lifecycle::Exited
                | Lifecycle::StartFailed(_)
                | Lifecycle::Destroyed => return Ok(()),
                Lifecycle::Starting => {
                    // Wake startup before waiting for its operation guard. The
                    // guard transfers actual termination to this stop caller.
                    self.stop_requested.send_replace(true);
                }
                Lifecycle::Running | Lifecycle::Stopping | Lifecycle::Destroying { .. } => {}
            }
        }
        // This marker's Drop must run after the operation lock is released.
        #[allow(clippy::collection_is_never_read)]
        let _activity;
        let _operation = self.operation.lock().await;
        _activity = self
            .registry
            .upgrade()
            .map(|registry| registry.operation(self.birth));
        if matches!(*self.state.lock().await, Lifecycle::Destroyed) {
            return Ok(());
        }
        let mut destroying = self.destroy_requested.subscribe();
        tokio::select! {
            outcome = self.stop_leader(mode) => outcome,
            _requested = destroying.wait_for(|requested| *requested) => Err(VmError::Destroyed),
        }
    }

    async fn wait(&self) -> Result<VmExit, VmError> {
        self.wait_for_terminal().await
    }

    async fn invoke_private_http(
        &self,
        request: vm_trait::PrivateHttpRequest,
    ) -> Result<vm_trait::PrivateHttpResponse, VmError> {
        if !self.private_http_enabled {
            return Err(VmError::Unsupported {
                feature: "private HTTP requires the declared http.v1 gateway handler contract"
                    .to_owned(),
                provider: PROVIDER_NAME.to_owned(),
            });
        }
        let request = private_http_request_message(request)?;
        let request_id = self
            .next_private_http_request
            .fetch_add(1, Ordering::Relaxed);
        if request_id == 0 {
            return Err(VmError::InvalidState(
                "private HTTP request identifier overflowed",
            ));
        }
        let (sender, receiver) = oneshot::channel();
        self.private_http_waiters
            .lock()
            .await
            .insert(request_id, sender);
        if let Err(error) = self
            .worker
            .request(WorkerCommand::InvokePrivateHttp {
                request_id,
                request,
            })
            .await
        {
            self.private_http_waiters.lock().await.remove(&request_id);
            return Err(error);
        }
        match timeout(PRIVATE_HTTP_TIMEOUT, receiver).await {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => Err(provider_error("private-http-response", error)),
            Err(_) => {
                self.private_http_waiters.lock().await.remove(&request_id);
                Err(unavailable_error(
                    "private HTTP handler",
                    "response timed out",
                ))
            }
        }
    }

    // The offer must remain owned by the timeout future so cancellation drops
    // it and releases its pending broker slot.
    #[allow(clippy::significant_drop_tightening)]
    async fn open_private_service_connection(
        &self,
    ) -> Result<BoxedPrivateServiceConnection, VmError> {
        let timeout_duration =
            self.private_service_timeout
                .ok_or_else(|| VmError::Unsupported {
                    feature: "private HTTP service is not declared for this VM".to_owned(),
                    provider: PROVIDER_NAME.to_owned(),
                })?;
        {
            let state = self.state.lock().await;
            match &*state {
                Lifecycle::Running => {}
                Lifecycle::Destroying { .. } | Lifecycle::Destroyed => {
                    return Err(VmError::Destroyed);
                }
                Lifecycle::Exited => {
                    return Err(VmError::InvalidState("the VM has exited"));
                }
                Lifecycle::Provisioned | Lifecycle::Starting => {
                    return Err(VmError::InvalidState("the VM is not running"));
                }
                Lifecycle::Stopping => {
                    return Err(VmError::InvalidState("the VM is stopping"));
                }
                Lifecycle::StartFailed(_) => {
                    return Err(VmError::InvalidState("the VM failed to start"));
                }
            }
        }
        let dispatch = self.private_service_dispatch.clone().ok_or_else(|| {
            unavailable_error("private service broker", "dispatch is unavailable")
        })?;
        let permit = dispatch.try_acquire_owned().map_err(|error| {
            unavailable_error(
                "private service connection",
                match error {
                    tokio::sync::TryAcquireError::Closed => "dispatch is closed",
                    tokio::sync::TryAcquireError::NoPermits => "connection capacity is exhausted",
                },
            )
        })?;
        let broker = {
            let resources = self.resources.lock().await;
            resources
                .as_ref()
                .and_then(|owned| owned.service_broker.as_ref())
                .cloned()
        }
        .ok_or_else(|| unavailable_error("private service broker", "broker is unavailable"))?;
        let offer = broker
            .reserve()
            .map_err(|error| unavailable_error("private service connection", error.to_string()))?;
        let connection = PrivateServiceConnectionMessage {
            connection_id: offer.id(),
            challenge: offer.challenge().clone(),
        };
        let worker = Arc::clone(&self.worker);
        let control_task = tokio::spawn(async move {
            let _permit = permit;
            worker
                .request(WorkerCommand::OpenPrivateServiceConnection { connection })
                .await
        });
        let result = timeout(timeout_duration, async {
            control_task
                .await
                .map_err(|error| provider_error("worker-control-task", error))??;
            offer
                .connect()
                .await
                .map_err(|error| unavailable_error("private service connection", error.to_string()))
        })
        .await;
        result.map_or_else(
            |_| {
                Err(unavailable_error(
                    "private service connection",
                    "connection timed out",
                ))
            },
            |result| result.map(|connection| Box::new(connection) as BoxedPrivateServiceConnection),
        )
    }

    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        let was_started = {
            let mut state = self.state.lock().await;
            let was_started = match &*state {
                Lifecycle::Destroyed => return Ok(()),
                Lifecycle::Destroying { was_started } => *was_started,
                Lifecycle::Provisioned | Lifecycle::StartFailed(_) => false,
                Lifecycle::Starting
                | Lifecycle::Running
                | Lifecycle::Stopping
                | Lifecycle::Exited => true,
            };
            // A request blocks further starts, but does not confirm cleanup.
            *state = Lifecycle::Destroying { was_started };
            self.destroy_requested.send_replace(true);
            drop(state);
            was_started
        };
        // This marker's Drop must run after the operation lock is released.
        #[allow(clippy::collection_is_never_read)]
        let _activity;
        let _operation = self.operation.lock().await;
        _activity = self
            .registry
            .upgrade()
            .map(|registry| registry.operation(self.birth));
        self.force_cleanup_locked(was_started).await?;
        *self.state.lock().await = Lifecycle::Destroyed;
        if !was_started {
            let _terminal = self.terminal_guard.lock().await;
            self.terminal
                .send_replace(Some(super::common::Terminal::Destroyed));
        }
        Ok(())
    }
}
