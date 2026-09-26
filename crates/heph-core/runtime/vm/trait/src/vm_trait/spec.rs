use std::{collections::BTreeMap, net::IpAddr, path::PathBuf, time::Duration};
use uuid::Uuid;

/// Fixed size of an opaque runtime-authority bearer credential.
pub const RUNTIME_AUTHORITY_CREDENTIAL_BYTES: usize = 32;
/// Fixed size of a separately discriminated runtime Git bearer.
pub const RUNTIME_GIT_CREDENTIAL_BYTES: usize = 32;

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
    /// Optional long-lived HTTP service exposed only through the provider's
    /// private host-to-guest transport.
    pub private_http_service: Option<PrivateHttpServiceSpec>,
    /// The initial command run inside the guest.
    pub command: GuestCommand,
    /// Sensitive one-run authority delivered through the authenticated guest
    /// bootstrap stream, never through environment variables or host mounts.
    pub runtime_authority: Option<RuntimeAuthorityBootstrap>,
    /// Optional internal runtime-Git bridge. The bridge is usable only when
    /// the authority bootstrap carries the separate runtime-Git credential.
    /// It never contains bearer material or a host socket path.
    pub runtime_git_bridge: Option<RuntimeGitBridge>,
    /// Caller-defined metadata associated with the VM.
    pub labels: BTreeMap<String, String>,
}

/// Non-secret guest-facing metadata for one exact runtime-Git bridge.
///
/// The provider maps the bridge to its configured host Unix socket over a
/// dedicated vsock port. The repository UUID is an opaque route component;
/// authentication and authorization remain host-side responsibilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeGitBridge {
    repository_id: Uuid,
    loopback_port: u16,
}

impl RuntimeGitBridge {
    /// Creates bridge metadata. Provider validation rejects nil IDs and
    /// loopback ports outside the unprivileged range.
    #[must_use]
    pub const fn new(repository_id: Uuid, loopback_port: u16) -> Self {
        Self {
            repository_id,
            loopback_port,
        }
    }

    /// Returns the opaque repository route component.
    #[must_use]
    pub const fn repository_id(self) -> Uuid {
        self.repository_id
    }

    /// Returns the guest loopback port used by the local proxy.
    #[must_use]
    pub const fn loopback_port(self) -> u16 {
        self.loopback_port
    }

    /// Returns the token-free URL convention installed in the worktree.
    #[must_use]
    pub fn remote_url(self) -> String {
        format!(
            "http://127.0.0.1:{}/{}",
            self.loopback_port, self.repository_id
        )
    }
}

/// Sensitive authority delivered once to trusted guest bootstrap code.
///
/// Debug output is deliberately redacted. Clones exist only because provider
/// specifications cross asynchronous worker boundaries; every dropped copy
/// is overwritten best-effort.
#[derive(Clone)]
pub struct RuntimeAuthorityBootstrap {
    session_id: Uuid,
    generation: u64,
    credential: [u8; RUNTIME_AUTHORITY_CREDENTIAL_BYTES],
    runtime_git_credential: Option<[u8; RUNTIME_GIT_CREDENTIAL_BYTES]>,
}

impl RuntimeAuthorityBootstrap {
    /// Creates an exact bootstrap payload. Providers reject generation zero.
    #[must_use]
    pub const fn new(
        session_id: Uuid,
        generation: u64,
        credential: [u8; RUNTIME_AUTHORITY_CREDENTIAL_BYTES],
    ) -> Self {
        Self {
            session_id,
            generation,
            credential,
            runtime_git_credential: None,
        }
    }

    /// Adds the separate exact-run Git bearer to the authenticated bootstrap.
    #[must_use]
    pub const fn with_runtime_git_credential(
        mut self,
        credential: [u8; RUNTIME_GIT_CREDENTIAL_BYTES],
    ) -> Self {
        self.runtime_git_credential = Some(credential);
        self
    }

    /// Returns the exact runtime session identifier.
    #[must_use]
    pub const fn session_id(&self) -> Uuid {
        self.session_id
    }

    /// Returns the exact issuance generation.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Exposes bearer bytes only to the provider's authenticated bootstrap
    /// transport conversion.
    #[must_use]
    pub const fn credential(&self) -> &[u8; RUNTIME_AUTHORITY_CREDENTIAL_BYTES] {
        &self.credential
    }

    /// Exposes the optional Git bearer only to authenticated bootstrap
    /// transport conversion.
    #[must_use]
    pub const fn runtime_git_credential(&self) -> Option<&[u8; RUNTIME_GIT_CREDENTIAL_BYTES]> {
        self.runtime_git_credential.as_ref()
    }
}

impl std::fmt::Debug for RuntimeAuthorityBootstrap {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeAuthorityBootstrap")
            .field("session_id", &self.session_id)
            .field("generation", &self.generation)
            .field("credential", &"[REDACTED]")
            .field(
                "runtime_git_credential",
                &self.runtime_git_credential.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

impl Drop for RuntimeAuthorityBootstrap {
    fn drop(&mut self) {
        for byte in &mut self.credential {
            *std::hint::black_box(byte) = 0;
        }
        if let Some(credential) = &mut self.runtime_git_credential {
            for byte in credential {
                *std::hint::black_box(byte) = 0;
            }
        }
    }
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
    /// The guest has no IP network and may use only the host secret broker
    /// through its dedicated provider transport.
    BrokerOnly,
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
    /// resolved nonzero value is returned in [`crate::VmEvent::Started`].
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

/// Configuration for a long-lived HTTP server bound to the guest loopback
/// interface.
#[derive(Debug, Clone)]
pub struct PrivateHttpServiceSpec {
    /// Guest loopback TCP port on which the released server listens.
    pub loopback_port: u16,
    /// Maximum number of provider-managed service connections in flight.
    pub max_connections: u32,
    /// Maximum time allowed to connect to the guest loopback server.
    pub connect_timeout: Duration,
}
