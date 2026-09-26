use std::{collections::BTreeMap, time::Duration};
use uuid::Uuid;

use super::PortForward;

/// A best-effort, live VM lifecycle event.
///
/// Event receivers can lag or disconnect. Consumers that require durable logs
/// must persist them outside the VM provider.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum VmEvent {
    /// The guest started running and its ingress assignments were resolved.
    Started {
        /// Effective forwarding rules, including provider-allocated host ports.
        ingress: Vec<PortForward>,
    },
    /// The guest bootstrap accepted the command and is ready to execute it.
    Ready,
    /// Trusted guest bootstrap persisted the exact runtime credential and
    /// acknowledged its session and issuance generation.
    RuntimeAuthorityAcknowledged {
        /// Exact runtime session identifier.
        session_id: Uuid,
        /// Exact issuance generation received by the guest.
        generation: u64,
    },
    /// The guest emitted output.
    Log {
        /// Output channel on which the bytes were emitted.
        stream: LogStream,
        /// Uninterpreted bytes emitted by the guest.
        bytes: Vec<u8>,
    },
    /// The guest reported a structured runtime metric.
    Metric(VmMetric),
    /// The guest handed its writable workspace back to the trusted host.
    FinalizeResult {
        /// Bounded human-readable result commit message.
        message: String,
    },
    /// The guest exited.
    Exited(VmExit),
}

/// A structured metric emitted by the guest runtime.
#[derive(Debug, Clone)]
pub struct VmMetric {
    /// Stable metric name.
    pub name: String,
    /// Numeric metric value.
    pub value: f64,
    /// Dimensions attached to the sample.
    pub labels: BTreeMap<String, String>,
}

/// A guest process output channel.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum LogStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

/// The termination status reported for a guest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmExit {
    /// Guest exit code, when one is available and `signal` is absent.
    pub code: Option<i32>,
    /// Signal that terminated the guest, when one is available and `code` is absent.
    pub signal: Option<i32>,
}

/// The requested strategy for stopping a running VM.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum StopMode {
    /// Ask the guest to stop and allow it a bounded amount of time to exit.
    Graceful {
        /// Maximum time to wait before the provider forces termination.
        timeout: Duration,
    },
    /// Terminate the VM without waiting for a graceful guest shutdown.
    Force,
}
