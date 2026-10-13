//! Existing weak event forwarder and original process/wall-clock monitors.
use crate::provider::{
    common::{
        Arc, LibkrunInstance, LogStream, VmEvent, VmExit, VmMetric, WireLogStream, WorkerEvent,
        broadcast, error, info, timeout, warn,
    },
    helpers::{send_event, to_port_forward},
    http::private_http_response,
};

impl LibkrunInstance {
    // The instance owns the worker's event sender. Retaining the instance while
    // waiting for receiver closure would keep both the sender and supervisor
    // alive forever after destruction. Process/terminal monitors retain live
    // VMs; upgrade this weak reference only while handling an actual event.
    pub(in crate::provider) fn spawn_event_forwarder(self: &Arc<Self>) {
        let instance = Arc::downgrade(self);
        let id = self.id.clone();
        let mut events = self.worker.subscribe_events();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        let Some(instance) = instance.upgrade() else {
                            break;
                        };
                        instance.handle_worker_event(event).await;
                    }
                    Err(broadcast::error::RecvError::Lagged(count)) => {
                        warn!(vm_id = %id.0, count, "worker event receiver lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    // The monitor tasks intentionally retain the instance until process exit.
    #[allow(clippy::significant_drop_tightening)]
    pub(in crate::provider) fn spawn_process_monitor(self: &Arc<Self>) {
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
    pub(in crate::provider) async fn handle_worker_event(&self, event: WorkerEvent) {
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
}
