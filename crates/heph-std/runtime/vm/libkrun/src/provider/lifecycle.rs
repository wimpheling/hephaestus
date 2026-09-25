use super::common::{
    Arc, LibkrunInstance, Lifecycle, LogStream, Terminal, VmError, VmEvent, VmExit, VmMetric,
    WireLogStream, WorkerCommand, WorkerEvent, broadcast, error, info, timeout, warn,
};
use super::{
    helpers::{cleanup_runtime, provider_error, send_event, to_port_forward, unavailable_error},
    http::private_http_response,
};

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

    // The task must own this Arc for the complete event-forwarding lifetime.
    #[allow(clippy::significant_drop_tightening)]
    pub(super) fn spawn_event_forwarder(self: &Arc<Self>) {
        let instance = Arc::clone(self);
        let mut events = instance.worker.subscribe_events();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => instance.handle_worker_event(event).await,
                    Err(broadcast::error::RecvError::Lagged(count)) => {
                        warn!(vm_id = %instance.id.0, count, "worker event receiver lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    // The monitor tasks intentionally retain the instance until process exit.
    #[allow(clippy::significant_drop_tightening)]
    pub(super) fn spawn_process_monitor(self: &Arc<Self>) {
        let instance = Arc::clone(self);
        let mut process_exit = instance.worker.subscribe_process_exit();
        tokio::spawn(async move {
            loop {
                let current = process_exit.borrow_and_update().clone();
                if let Some(status) = current {
                    instance.complete_exit(status.into_vm_exit()).await;
                    break;
                }
                if process_exit.changed().await.is_err() {
                    break;
                }
            }
        });

        // A configured private service is a supervisor-owned long-lived VM.
        // Its lifetime ends through explicit stop/destroy or worker exit, so
        // the one-shot wall-clock guard must not kill it while it is serving.
        if self.private_service_timeout.is_some() {
            return;
        }

        let instance = Arc::clone(self);
        tokio::spawn(async move {
            let mut terminal = instance.terminal.subscribe();
            let completed = timeout(instance.config.limits.wall_clock_timeout, async {
                loop {
                    if terminal.borrow_and_update().is_some() {
                        break;
                    }
                    if terminal.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await
            .is_ok();
            if !completed {
                warn!(vm_id = %instance.id.0, "wall-clock limit reached");
                let _kill_result = instance.worker.kill().await;
            }
        });
    }

    // One exhaustive match keeps the worker protocol's state transitions
    // auditable beside each decoded event variant.
    #[allow(clippy::cognitive_complexity)]
    pub(super) async fn handle_worker_event(&self, event: WorkerEvent) {
        match event {
            WorkerEvent::Started {
                ingress,
                vmm_pid,
                passt_pid,
            } => {
                info!(
                    vm_id = %self.id.0,
                    vmm_pid,
                    ?passt_pid,
                    forwards = ingress.len(),
                    "microVM started"
                );
                send_event(
                    &self.events,
                    VmEvent::Started {
                        ingress: ingress.into_iter().map(to_port_forward).collect(),
                    },
                );
            }
            WorkerEvent::Ready => {
                self.ready.send_replace(true);
                info!(vm_id = %self.id.0, "guest ready");
                send_event(&self.events, VmEvent::Ready);
            }
            WorkerEvent::RuntimeAuthorityAcknowledged {
                session_id,
                generation,
            } => {
                send_event(
                    &self.events,
                    VmEvent::RuntimeAuthorityAcknowledged {
                        session_id,
                        generation,
                    },
                );
            }
            WorkerEvent::Log { stream, bytes } => {
                let stream = match stream {
                    WireLogStream::Stdout => LogStream::Stdout,
                    WireLogStream::Stderr => LogStream::Stderr,
                };
                send_event(&self.events, VmEvent::Log { stream, bytes });
            }
            WorkerEvent::Metric {
                name,
                value,
                labels,
            } => send_event(
                &self.events,
                VmEvent::Metric(VmMetric {
                    name,
                    value,
                    labels,
                }),
            ),
            WorkerEvent::Health { nonce } => {
                tracing::debug!(vm_id = %self.id.0, nonce, "guest health response");
            }
            WorkerEvent::FinalizeResult { message } => {
                send_event(&self.events, VmEvent::FinalizeResult { message });
            }
            WorkerEvent::Exited { code, signal } => {
                self.complete_exit(VmExit { code, signal }).await;
            }
            WorkerEvent::BackendFailure(failure) => {
                error!(
                    vm_id = %self.id.0,
                    code = %failure.code,
                    message = %failure.message,
                    "worker backend failure"
                );
                self.fail_private_http_waiters("guest control channel failed")
                    .await;
            }
            WorkerEvent::PrivateHttpResponse {
                request_id,
                response,
            } => {
                let response = private_http_response(response);
                let waiter = self.private_http_waiters.lock().await.remove(&request_id);
                if let Some(waiter) = waiter {
                    let _sent = waiter.send(response);
                } else {
                    warn!(vm_id = %self.id.0, request_id, "discarding unmatched private HTTP response");
                }
            }
        }
    }

    pub(super) async fn complete_exit(&self, exit: VmExit) {
        let _guard = self.terminal_guard.lock().await;
        if self.terminal.borrow().is_some() {
            return;
        }
        self.terminal
            .send_replace(Some(Terminal::Exited(exit.clone())));
        {
            let mut state = self.state.lock().await;
            if !matches!(*state, Lifecycle::Destroyed) {
                *state = Lifecycle::Exited;
            }
        }
        send_event(&self.events, VmEvent::Exited(exit));
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
        self.shutdown_service_broker().await;
        if was_started {
            self.worker.kill().await?;
            let _status = timeout(self.config.startup_timeout, self.worker.wait_process())
                .await
                .map_err(|_| unavailable_error("worker cleanup", "worker reap timed out"))??;
        } else {
            let guard = self.terminal_guard.lock().await;
            if self.terminal.borrow().is_none() {
                self.terminal.send_replace(Some(Terminal::Destroyed));
            }
            drop(guard);
            let _destroy_result = self.worker.request(WorkerCommand::Destroy).await;
            let _status = timeout(self.config.startup_timeout, self.worker.wait_process())
                .await
                .map_err(|_| unavailable_error("worker cleanup", "worker reap timed out"))??;
        }

        self.cleanup_resources().await
    }

    // The broker is cloned out before awaiting shutdown so the resources lock
    // never spans an await.
    #[allow(clippy::significant_drop_tightening)]
    pub(super) async fn cleanup_resources(&self) -> Result<(), VmError> {
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
            cleanup_runtime(&owned.runtime_dir)?;
            owned.cgroup.cleanup()?;
            info!(
                vm_id = %self.id.0,
                runtime_dir = %owned.runtime_dir.display(),
                "VM resources cleaned"
            );
        }
        resources.take();
        drop(resources);
        self.provider_ids.ids.lock().await.remove(&self.id);
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
