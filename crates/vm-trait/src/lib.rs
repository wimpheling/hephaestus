//! Provider-neutral virtual machine abstractions for the Hephaestus runtime.

use async_trait::async_trait;
use std::{
    collections::BTreeMap, error::Error, net::IpAddr, path::PathBuf, sync::Arc, time::Duration,
};
use tokio::sync::broadcast;

/// A stable identifier for a virtual machine.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VmId(
    /// The provider-independent identifier value.
    pub String,
);

/// The complete, provider-neutral configuration used to provision a VM.
///
/// Every host path reachable from this specification is caller-owned. A
/// provider may open, attach, mount, stage, and detach these resources, but it
/// must never delete, reformat, truncate, or otherwise remove the supplied
/// root, disk, or mount backing paths. Provider-created overlays, uploads, and
/// runtime files remain provider-owned.
#[derive(Debug, Clone)]
pub struct VmSpec {
    /// The identifier to assign to the VM.
    pub id: VmId,
    /// The filesystem from which the guest boots.
    pub root: RootFilesystem,
    /// Additional block devices exposed to the guest.
    pub disks: Vec<VmDisk>,
    /// Host directories exposed inside the guest.
    pub mounts: Vec<VmMount>,
    /// Compute resources assigned to the guest.
    pub resources: VmResources,
    /// Guest network connectivity.
    pub network: NetworkMode,
    /// The initial command run inside the guest.
    pub command: GuestCommand,
    /// Caller-defined metadata associated with the VM.
    pub labels: BTreeMap<String, String>,
}

/// The host-backed filesystem from which a guest boots.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum RootFilesystem {
    /// A directory-backed root filesystem.
    Directory {
        /// Path to the directory on the host.
        host_path: PathBuf,
    },
    /// A disk-image-backed root filesystem.
    Disk {
        /// Path to the disk image on the host.
        host_path: PathBuf,
        /// On-disk representation of the image.
        format: DiskFormat,
        /// Whether the guest receives read-only access to the image.
        read_only: bool,
    },
}

/// An explicitly declared disk image format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiskFormat {
    /// A raw block-device image.
    Raw,
    /// A QEMU copy-on-write version 2 image.
    Qcow2,
}

/// An additional host-backed disk exposed to a guest.
#[derive(Debug, Clone)]
pub struct VmDisk {
    /// Identifier used to distinguish the disk within the VM specification.
    pub id: String,
    /// Path to the disk image on the host.
    pub host_path: PathBuf,
    /// On-disk representation of the image.
    pub format: DiskFormat,
    /// Whether the guest receives read-only access to the disk.
    pub read_only: bool,
}

/// A host directory exposed at a path inside a guest.
#[derive(Debug, Clone)]
pub struct VmMount {
    /// Provider-independent name by which the mount is identified.
    pub tag: String,
    /// Path to the directory on the host.
    pub host_path: PathBuf,
    /// Path at which the directory is made available inside the guest.
    pub guest_path: PathBuf,
    /// Whether the guest receives read-only access to the directory.
    pub read_only: bool,
}

/// Compute resources assigned to a guest.
#[derive(Debug, Clone)]
pub struct VmResources {
    /// Number of virtual CPUs assigned to the guest.
    pub vcpus: u8,
    /// Amount of guest memory, in mebibytes.
    pub memory_mib: u32,
}

/// Network connectivity available to a guest.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum NetworkMode {
    /// The guest has no network connectivity.
    Disabled,
    /// The provider supplies user-mode networking.
    UserMode {
        /// Host ports forwarded to ports inside the guest.
        ingress: Vec<PortForward>,
    },
}

/// Transport protocol used by an ingress forwarding rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PortProtocol {
    /// Transmission Control Protocol.
    Tcp,
}

/// A port forwarded from the host to a guest.
#[derive(Debug, Clone)]
pub struct PortForward {
    /// Transport protocol accepted by the forwarding rule.
    pub protocol: PortProtocol,
    /// Host address on which the provider binds the forwarding rule.
    pub bind_addr: IpAddr,
    /// Port bound on the host.
    ///
    /// A value of zero asks the provider to allocate an available port. The
    /// resolved nonzero value is returned in [`VmEvent::Started`].
    pub host_port: u16,
    /// Port receiving forwarded traffic inside the guest.
    pub guest_port: u16,
}

/// The initial command run inside a guest.
#[derive(Debug, Clone)]
pub struct GuestCommand {
    /// Absolute guest path of the program to execute directly, without a shell.
    pub program: String,
    /// Arguments passed to the program, excluding the program itself.
    pub args: Vec<String>,
    /// Environment variables made available to the program.
    pub env: BTreeMap<String, String>,
    /// Absolute working directory inside the guest.
    pub working_dir: Option<PathBuf>,
}

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
    /// The guest emitted output.
    Log {
        /// Output channel on which the bytes were emitted.
        stream: LogStream,
        /// Uninterpreted bytes emitted by the guest.
        bytes: Vec<u8>,
    },
    /// The guest reported a structured runtime metric.
    Metric(VmMetric),
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

/// An error returned by a VM provider or instance.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VmError {
    /// A VM with the requested identifier already exists.
    #[error("VM already exists: {0:?}")]
    AlreadyExists(VmId),

    /// A field in the requested VM specification is invalid.
    #[error("invalid VM specification field {field:?}: {reason}")]
    InvalidSpec {
        /// Field path within the VM specification.
        field: String,
        /// Human-readable validation failure.
        reason: String,
    },

    /// The selected provider does not support a requested feature.
    #[error("provider {provider:?} does not support {feature}")]
    Unsupported {
        /// Unsupported provider-neutral feature.
        feature: String,
        /// Provider that rejected the feature.
        provider: String,
    },

    /// The VM was destroyed without producing an exit status.
    #[error("VM was destroyed before it produced an exit status")]
    Destroyed,

    /// A required runtime resource is temporarily unavailable.
    #[error("resource {resource:?} is unavailable: {reason}")]
    Unavailable {
        /// Resource that could not be acquired.
        resource: String,
        /// Human-readable reason the resource is unavailable.
        reason: String,
    },

    /// The requested operation is not valid in the VM's current lifecycle state.
    #[error("VM is in an invalid state: {0}")]
    InvalidState(&'static str),

    /// An unexpected provider-specific failure.
    #[error("unexpected {provider} provider error ({code}): {source}")]
    Provider {
        /// Provider that failed.
        provider: String,
        /// Stable provider-specific diagnostic code.
        code: String,
        /// Original error returned by the provider backend.
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },
}

/// Provisions virtual machines using a particular backend.
///
/// Provisioning allocates the resources described by a [`VmSpec`] but does not
/// start the guest. Call [`VmInstance::start`] explicitly after provisioning.
#[async_trait]
pub trait VmProvider: Send + Sync + 'static {
    /// Returns the stable name of this provider implementation.
    fn name(&self) -> &'static str;

    /// Allocates a stopped VM from `spec`.
    ///
    /// A failed provisioning attempt must detach all caller-owned resources
    /// and clean up provider-owned resources before returning. It must preserve
    /// every caller-owned path in `spec`.
    ///
    /// # Errors
    ///
    /// Returns [`VmError::AlreadyExists`] when the identifier is already in
    /// use, or another [`VmError`] when resource allocation fails.
    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError>;

    /// Confirms that resources belonging to an abandoned VM identifier are
    /// destroyed after a supervisor restart.
    ///
    /// This operation is idempotent. It may terminate orphaned workers and
    /// remove provider-owned runtime resources, but it must preserve every
    /// caller-owned path previously supplied through a [`VmSpec`]. Successful
    /// completion confirms that disks are detached and the identifier can be
    /// safely reused.
    ///
    /// # Errors
    ///
    /// Returns an error when complete cleanup and disk detachment cannot be
    /// confirmed.
    async fn cleanup_orphan(&self, id: &VmId) -> Result<(), VmError>;
}

/// A provisioned VM with an explicit lifecycle.
///
/// Instances are returned in a provisioned state. They are one-shot, must be
/// started explicitly, and must be destroyed when their resources are no
/// longer required. Dropping a handle does not release provider resources.
#[async_trait]
pub trait VmInstance: Send + Sync + 'static {
    /// Returns the stable identifier assigned during provisioning.
    fn id(&self) -> &VmId;

    /// Starts the guest, or joins an in-progress start.
    ///
    /// This operation is idempotent while the VM is starting or running. An
    /// exited VM cannot be restarted.
    ///
    /// # Errors
    ///
    /// Returns [`VmError::InvalidState`] if the instance cannot be started
    /// from its current state, or another [`VmError`] if startup fails.
    async fn start(&self) -> Result<(), VmError>;

    /// Stops the guest using the requested strategy.
    ///
    /// This operation is idempotent. A provisioned, stopped, or exited VM
    /// returns successfully. Graceful stop requests guest cancellation and
    /// force-stops the VM after its timeout.
    ///
    /// # Errors
    ///
    /// Returns [`VmError::InvalidState`] if the instance cannot be stopped
    /// from its current state, or another [`VmError`] if shutdown fails.
    async fn stop(&self, mode: StopMode) -> Result<(), VmError>;

    /// Waits until the guest exits and returns its cached termination status.
    ///
    /// Any number of callers may wait concurrently or after exit. Every
    /// successful caller receives the same [`VmExit`]. A VM destroyed before
    /// startup returns [`VmError::Destroyed`].
    ///
    /// # Errors
    ///
    /// Returns [`VmError::InvalidState`] if waiting is not valid in the
    /// instance's current state, or another [`VmError`] if monitoring fails.
    async fn wait(&self) -> Result<VmExit, VmError>;

    /// Subscribes to best-effort live lifecycle and log events.
    ///
    /// The returned receiver reports lag and channel closure using Tokio's
    /// broadcast receiver errors. Events emitted before subscription are not
    /// replayed.
    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent>;

    /// Releases all resources owned by the instance.
    ///
    /// This operation is idempotent and force-terminates a running VM. If an
    /// exit status was already cached, destroying the VM does not discard it.
    /// Successful completion confirms that all disks and mounts are detached.
    /// Caller-owned root, disk, and mount backing paths are always preserved;
    /// only provider-owned runtime resources are removed.
    ///
    /// # Errors
    ///
    /// Returns a [`VmError`] if complete cleanup cannot be confirmed.
    async fn destroy(&self) -> Result<(), VmError>;
}

#[cfg(test)]
mod tests {
    use super::{VmInstance, VmProvider};

    fn assert_runtime_trait<T: ?Sized + Send + Sync + 'static>() {}

    #[test]
    fn provider_traits_are_object_safe() {
        assert_runtime_trait::<dyn VmProvider>();
        assert_runtime_trait::<dyn VmInstance>();
    }
}
