//! Each admitted lifecycle request owns its exact instance until completion.
//! Caller cancellation drops only the result receiver, never the operation task.
use super::common::{ErrorSnapshot, LibkrunInstance, Lifecycle, StopMode, VmError, info, oneshot};
use super::{helpers::provider_error, http::wait_start_result};

pub(super) enum Request {
    Start,
    Stop(StopMode),
    Destroy,
}

impl LibkrunInstance {
    pub(super) async fn run_lifecycle(&self, request: Request) -> Result<(), VmError> {
        let instance = self.this.upgrade().ok_or(VmError::Destroyed)?;
        let activity = self
            .registry
            .upgrade()
            .map(|registry| registry.lifecycle_task(self.birth))
            .transpose()?;
        let (completed, outcome) = oneshot::channel();
        // No await precedes this handoff and no abort handle is exposed. The
        // actual instance retains backend, pinned roots and original lifetime.
        tokio::spawn(async move {
            let result = match request {
                Request::Start => instance.start_owned().await,
                Request::Stop(mode) => instance.stop_owned(mode).await,
                Request::Destroy => instance.destroy_owned().await,
            };
            // A lost receiver is not termination/cleanup evidence. The task
            // reaches the same success or held/error outcome independently.
            drop(instance);
            drop(activity);
            let _reported = completed.send(result);
        });
        outcome
            .await
            .map_err(|error| provider_error("lifecycle-task", error))?
    }

    async fn start_admission(&self) -> Result<Option<super::registry::Operation>, VmError> {
        let registry = self.registry.upgrade();
        if let Some(registry) = registry {
            Ok(Some(
                registry.admit_start(&self.provider_ids, self.birth).await?,
            ))
        } else {
            // Only isolated unit fixtures lack a provider-admitted birth.
            #[cfg(test)]
            if self.birth != 0 {
                return Err(VmError::Destroyed);
            }
            #[cfg(not(test))]
            return Err(VmError::Destroyed);
            #[cfg(test)]
            Ok(None)
        }
    }

    async fn start_owned(&self) -> Result<(), VmError> {
        // This marker's Drop must run after the operation lock is released.
        #[allow(clippy::collection_is_never_read)]
        let _activity;
        let _operation = self.operation.lock().await;
        _activity = self.start_admission().await?;
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

    async fn stop_owned(&self, mode: StopMode) -> Result<(), VmError> {
        {
            let state = self.state.lock().await;
            match &*state {
                Lifecycle::Provisioned
                | Lifecycle::Exited
                | Lifecycle::StartFailed(_)
                | Lifecycle::Destroyed => return Ok(()),
                Lifecycle::Starting => {
                    // Wake startup before waiting for its operation guard. The
                    // guard transfers actual termination to this owned stop task.
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
        let outcome = tokio::select! {
            outcome = self.stop_leader(mode) => outcome,
            _requested = destroying.wait_for(|requested| *requested) => Err(VmError::Destroyed),
        };
        if let (Some(registry), Err(error)) = (self.registry.upgrade(), &outcome) {
            // Record termination failure under the operation guard, even when
            // its caller has gone. A later successful destroy clears it.
            registry.cleanup_failed(self.birth, error);
        }
        outcome
    }

    async fn destroy_owned(&self) -> Result<(), VmError> {
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
