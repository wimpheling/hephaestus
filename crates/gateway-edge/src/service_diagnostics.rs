//! Bounded, content-free diagnostics for one persistent service instance.

use tokio::sync::{broadcast, watch};
use vm_trait::{LogStream, VmEvent};

use crate::ServiceWorkerState;

/// Safe lifecycle and output accounting observed from one service VM.
///
/// Guest log and metric contents are intentionally discarded. In particular,
/// this snapshot cannot contain request headers, request bodies, credentials,
/// or arbitrary guest-provided metric labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceDiagnosticsSnapshot {
    /// Current parent-owned worker lifecycle state.
    pub worker_state: ServiceWorkerState,
    /// Lifecycle milestones reported by the provider.
    pub lifecycle: ServiceLifecycleEvidence,
    /// Saturating count of stdout bytes emitted by the guest.
    pub stdout_bytes: u64,
    /// Saturating count of stderr bytes emitted by the guest.
    pub stderr_bytes: u64,
    /// Saturating count of events skipped because the receiver lagged.
    pub lagged_events: u64,
    /// Whether the provider event channel closed.
    pub channel_closed: bool,
}

impl Default for ServiceDiagnosticsSnapshot {
    fn default() -> Self {
        Self {
            worker_state: ServiceWorkerState::Provisioned,
            lifecycle: ServiceLifecycleEvidence::default(),
            stdout_bytes: 0,
            stderr_bytes: 0,
            lagged_events: 0,
            channel_closed: false,
        }
    }
}

/// Lifecycle milestones observed from the provider event stream.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ServiceLifecycleEvidence(u8);

impl ServiceLifecycleEvidence {
    const STARTED: u8 = 1;
    const READY: u8 = 1 << 1;
    const EXITED: u8 = 1 << 2;

    /// Returns whether the provider reported that the VM started.
    #[must_use]
    pub const fn started(self) -> bool {
        self.0 & Self::STARTED != 0
    }

    /// Returns whether the provider reported that guest bootstrap became ready.
    #[must_use]
    pub const fn ready(self) -> bool {
        self.0 & Self::READY != 0
    }

    /// Returns whether the provider reported that the VM exited.
    #[must_use]
    pub const fn exited(self) -> bool {
        self.0 & Self::EXITED != 0
    }

    const fn mark_started(&mut self) {
        self.0 |= Self::STARTED;
    }

    const fn mark_ready(&mut self) {
        self.0 |= Self::READY;
    }

    const fn mark_exited(&mut self) {
        self.0 |= Self::EXITED;
    }
}

/// Parent-owned event accounting for one service worker.
pub struct ServiceDiagnostics {
    snapshot: ServiceDiagnosticsSnapshot,
    updates: watch::Sender<ServiceDiagnosticsSnapshot>,
}

impl ServiceDiagnostics {
    /// Creates an event collector publishing into a diagnostics watch channel.
    pub fn new(updates: watch::Sender<ServiceDiagnosticsSnapshot>) -> Self {
        Self {
            snapshot: ServiceDiagnosticsSnapshot::default(),
            updates,
        }
    }

    /// Records one provider event and returns whether its channel remains open.
    pub fn observe(&mut self, event: Result<VmEvent, broadcast::error::RecvError>) -> bool {
        match event {
            Ok(event) => self.observe_event(event),
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                self.snapshot.lagged_events = self.snapshot.lagged_events.saturating_add(skipped);
                self.publish();
                true
            }
            Err(broadcast::error::RecvError::Closed) => {
                self.snapshot.channel_closed = true;
                self.publish();
                false
            }
        }
    }

    fn observe_event(&mut self, event: VmEvent) -> bool {
        match event {
            VmEvent::Started { .. } => self.snapshot.lifecycle.mark_started(),
            VmEvent::Ready => self.snapshot.lifecycle.mark_ready(),
            VmEvent::Exited(_) => self.snapshot.lifecycle.mark_exited(),
            VmEvent::Log { stream, bytes } => {
                let count = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                match stream {
                    LogStream::Stdout => {
                        self.snapshot.stdout_bytes =
                            self.snapshot.stdout_bytes.saturating_add(count);
                    }
                    LogStream::Stderr => {
                        self.snapshot.stderr_bytes =
                            self.snapshot.stderr_bytes.saturating_add(count);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        self.publish();
        true
    }

    fn publish(&self) {
        let worker_state = self.updates.borrow().worker_state;
        let mut snapshot = self.snapshot;
        snapshot.worker_state = worker_state;
        self.updates.send_replace(snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast;
    use vm_trait::{LogStream, VmExit};

    #[test]
    fn records_only_lifecycle_and_saturating_byte_counts() {
        let (updates, receiver) = watch::channel(ServiceDiagnosticsSnapshot::default());
        let mut diagnostics = ServiceDiagnostics::new(updates);
        assert!(diagnostics.observe(Ok(VmEvent::Started {
            ingress: Vec::new()
        })));
        assert!(diagnostics.observe(Ok(VmEvent::Ready)));
        assert!(diagnostics.observe(Ok(VmEvent::Log {
            stream: LogStream::Stdout,
            bytes: b"Authorization: Bearer secret".to_vec(),
        })));
        assert!(diagnostics.observe(Ok(VmEvent::Log {
            stream: LogStream::Stderr,
            bytes: b"secret-body".to_vec(),
        })));
        assert!(diagnostics.observe(Ok(VmEvent::Exited(VmExit {
            code: Some(0),
            signal: None,
        }))));
        let snapshot = *receiver.borrow();
        assert_eq!(snapshot.stdout_bytes, 28);
        assert_eq!(snapshot.stderr_bytes, 11);
        assert!(snapshot.lifecycle.started());
        assert!(snapshot.lifecycle.ready());
        assert!(snapshot.lifecycle.exited());
        assert!(!format!("{snapshot:?}").contains("secret"));
    }

    #[test]
    fn lag_is_saturating_and_channel_close_is_terminal() {
        let (updates, receiver) = watch::channel(ServiceDiagnosticsSnapshot::default());
        let mut diagnostics = ServiceDiagnostics::new(updates);
        diagnostics.snapshot.lagged_events = u64::MAX - 1;
        assert!(diagnostics.observe(Err(broadcast::error::RecvError::Lagged(3))));
        assert_eq!(receiver.borrow().lagged_events, u64::MAX);
        assert!(!diagnostics.observe(Err(broadcast::error::RecvError::Closed)));
        assert!(receiver.borrow().channel_closed);
    }

    #[test]
    fn output_byte_counts_are_saturating() {
        let (updates, receiver) = watch::channel(ServiceDiagnosticsSnapshot::default());
        let mut diagnostics = ServiceDiagnostics::new(updates);
        diagnostics.snapshot.stdout_bytes = u64::MAX - 1;
        diagnostics.snapshot.stderr_bytes = u64::MAX - 1;
        assert!(diagnostics.observe(Ok(VmEvent::Log {
            stream: LogStream::Stdout,
            bytes: vec![0; 3],
        })));
        assert!(diagnostics.observe(Ok(VmEvent::Log {
            stream: LogStream::Stderr,
            bytes: vec![0; 3],
        })));
        assert_eq!(receiver.borrow().stdout_bytes, u64::MAX);
        assert_eq!(receiver.borrow().stderr_bytes, u64::MAX);
    }
}
