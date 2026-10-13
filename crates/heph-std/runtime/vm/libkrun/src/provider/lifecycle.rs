#[path = "lifecycle/events.rs"]
mod events;
use super::common::{
    LibkrunInstance, Lifecycle, Ordering, PROVIDER_NAME, StopMode, Terminal, VmError, VmEvent,
    VmExit, WorkerCommand, info, timeout, warn,
};
use super::helpers::{cleanup_runtime, provider_error, send_event, unavailable_error};

impl LibkrunInstance {
    pub(super) async fn start_leader(&self) -> Result<(), VmError> {
        timeout(
            self.config.startup_timeout,
            self.worker.request(WorkerCommand::Start),
        )
        .await
        .map_err(|_| unavailable_error("worker startup", "startup request timed out"))??;

        let mut ready = self.ready.subscribe();
        let mut terminal = self.terminal.subscribe();
        timeout(self.config.readiness_timeout, async {
            loop {
                if *ready.borrow_and_update() {
                    return Ok(());
                }
                let current_terminal = terminal.borrow_and_update().clone();
                if let Some(outcome) = current_terminal {
                    return match outcome {
                        Terminal::Exited(exit) => Err(unavailable_error(
                            "guest readiness",
                            format!("guest exited before readiness: {exit:?}"),
                        )),
                        Terminal::Destroyed => Err(VmError::Destroyed),
                    };
                }
                tokio::select! {
                    result = ready.changed() => {
                        result.map_err(|error| provider_error("ready-channel", error))?;
                    }
                    result = terminal.changed() => {
                        result.map_err(|error| provider_error("terminal-channel", error))?;
                    }
                }
            }
        })
        .await
        .map_err(|_| unavailable_error("guest readiness", "readiness timeout elapsed"))?
    }

    pub(super) async fn stop_leader(&self, mode: StopMode) -> Result<(), VmError> {
        {
            let mut state = self.state.lock().await;
            match &*state {
                Lifecycle::Provisioned
                | Lifecycle::Exited
                | Lifecycle::StartFailed(_)
                | Lifecycle::Destroyed => return Ok(()),
                Lifecycle::Destroying { .. } => return Err(VmError::Destroyed),
                // The operation mutex excludes a still-running stop. A prior
                // failed kill must be retried with the same actual backend.
                Lifecycle::Stopping | Lifecycle::Starting | Lifecycle::Running => {
                    *state = Lifecycle::Stopping;
                }
            }
        }

        match mode {
            StopMode::Graceful { timeout: grace } => {
                let timeout_ms = u64::try_from(grace.as_millis()).unwrap_or(u64::MAX);
                // The grace budget covers the control request as well as
                // guest exit; a blocked Cancel must still reach kill fallback.
                let graceful = timeout(grace, async {
                    let cancel = self
                        .worker
                        .request(WorkerCommand::Cancel { timeout_ms })
                        .await;
                    if let Err(error) = cancel {
                        warn!(vm_id = %self.id.0, %error, "guest cancellation request failed");
                    }
                    self.wait_for_terminal().await
                })
                .await;
                if graceful.is_err() {
                    warn!(vm_id = %self.id.0, "graceful stop timed out; killing worker");
                    self.worker.kill().await?;
                }
            }
            StopMode::Force => self.worker.kill().await?,
            _ => {
                return Err(VmError::Unsupported {
                    feature: "stop mode".to_owned(),
                    provider: PROVIDER_NAME.to_owned(),
                });
            }
        }
        self.wait_for_terminal().await.map(|_| ())
    }

    pub(super) async fn wait_for_terminal(&self) -> Result<VmExit, VmError> {
        let mut terminal = self.terminal.subscribe();
        loop {
            let current = terminal.borrow_and_update().clone();
            if let Some(outcome) = current {
                return match outcome {
                    Terminal::Exited(exit) => Ok(exit),
                    Terminal::Destroyed => Err(VmError::Destroyed),
                };
            }
            terminal
                .changed()
                .await
                .map_err(|error| provider_error("terminal-channel", error))?;
        }
    }

    pub(super) async fn complete_exit(&self, exit: VmExit) {
        let _guard = self.terminal_guard.lock().await;
        if self.terminal.borrow().is_some() {
            return;
        }
        let publish_exit = {
            let mut state = self.state.lock().await;
            // The worker's actual reap remains available to cleanup, but a VM
            // destroyed before Start has no guest exit to publish to waiters.
            let publish_exit = !matches!(*state, Lifecycle::Destroying { was_started: false });
            // Cleanup's process exit must not erase the cached startup failure
            // that callers serialized behind the original start must receive.
            if !matches!(
                *state,
                Lifecycle::StartFailed(_) | Lifecycle::Destroying { .. } | Lifecycle::Destroyed
            ) {
                *state = Lifecycle::Exited;
            }
            drop(state);
            publish_exit
        };
        if publish_exit {
            self.terminal
                .send_replace(Some(Terminal::Exited(exit.clone())));
            send_event(&self.events, VmEvent::Exited(exit));
        }
        self.shutdown_service_broker().await;
        self.fail_private_http_waiters("guest exited before private HTTP response")
            .await;
    }

    pub(super) async fn fail_private_http_waiters(&self, reason: &str) {
        let waiters = std::mem::take(&mut *self.private_http_waiters.lock().await);
        for (_, waiter) in waiters {
            let _sent = waiter.send(Err(unavailable_error("private HTTP handler", reason)));
        }
    }

    pub(super) async fn force_cleanup(&self, was_started: bool) -> Result<(), VmError> {
        // This marker's Drop must run after the operation lock is released.
        #[allow(clippy::collection_is_never_read)]
        let _activity;
        let _operation = self.operation.lock().await;
        _activity = self
            .registry
            .upgrade()
            .map(|registry| registry.operation(self.birth));
        self.force_cleanup_locked(was_started).await
    }

    // Every caller holds the per-instance operation lock through confirmed reap
    // and allocation cleanup. An old cleaned handle must not touch a reused ID.
    pub(super) async fn force_cleanup_locked(&self, was_started: bool) -> Result<(), VmError> {
        if self.cleanup_complete.load(Ordering::Acquire) {
            return Ok(());
        }
        let result = self.terminate_and_cleanup(was_started).await;
        if let Err(error) = &result {
            if let Some(registry) = self.registry.upgrade() {
                registry.cleanup_failed(self.birth, error);
            }
        }
        result
    }

    async fn terminate_and_cleanup(&self, was_started: bool) -> Result<(), VmError> {
        self.shutdown_service_broker().await;
        if !was_started {
            // Graceful protocol destruction is only a hint; kill plus actual
            // reap below establish the worker's absence even if it fails.
            let _destroy_result = timeout(
                self.config.startup_timeout,
                self.worker.request(WorkerCommand::Destroy),
            )
            .await;
        }
        self.worker.kill().await?;
        let _status = timeout(self.config.startup_timeout, self.worker.wait_process())
            .await
            .map_err(|_| unavailable_error("worker cleanup", "worker reap timed out"))??;
        self.cleanup_resources().await
    }

    // The broker is cloned out before awaiting shutdown so the resources lock
    // never spans an await.
    #[allow(clippy::significant_drop_tightening)]
    pub(super) async fn cleanup_resources(&self) -> Result<(), VmError> {
        if self.cleanup_complete.load(Ordering::Acquire) {
            return Ok(());
        }
        let broker = self
            .resources
            .lock()
            .await
            .as_ref()
            .and_then(|owned| owned.service_broker.as_ref())
            .cloned();
        if let Some(broker) = broker {
            broker.shutdown().await;
        }
        let mut resources = self.resources.lock().await;
        if let Some(owned) = resources.as_ref() {
            if let Some(runtime) = owned.runtime_child.as_ref() {
                owned.cgroup.cleanup()?;
                runtime
                    .remove(true)
                    .map_err(|error| provider_error("runtime-cleanup", error))?;
            } else if let Some(roots) = owned.cleanup_roots.as_ref() {
                roots.cleanup(&self.config, &self.id)?;
            } else {
                owned.cgroup.cleanup()?;
                cleanup_runtime(&owned.runtime_dir)?;
            }
            info!(
                vm_id = %self.id.0,
                runtime_dir = %owned.runtime_dir.display(),
                "VM resources cleaned"
            );
        }
        if let Some(record) = resources
            .as_ref()
            .and_then(|owned| owned.birth_record.as_ref())
        {
            record.remove_original()?;
        }
        resources.take();
        drop(resources);
        let mut ids = self.provider_ids.ids.lock().await;
        ids.remove(&self.id);
        self.cleanup_complete.store(true, Ordering::Release);
        if let Some(registry) = self.registry.upgrade() {
            registry.cleaned(self.birth);
        }
        drop(ids);
        Ok(())
    }

    pub(super) async fn shutdown_service_broker(&self) {
        let broker = self
            .resources
            .lock()
            .await
            .as_ref()
            .and_then(|owned| owned.service_broker.as_ref())
            .cloned();
        if let Some(broker) = broker {
            broker.shutdown().await;
        }
    }
}
