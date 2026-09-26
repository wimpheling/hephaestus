use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, net::IpAddr, path::PathBuf};
use uuid::Uuid;
use vm_trait::RUNTIME_AUTHORITY_CREDENTIAL_BYTES;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedSpec {
    pub id: String,
    pub root: PreparedRoot,
    pub disks: Vec<PreparedDisk>,
    pub mounts: Vec<PreparedMount>,
    pub vcpus: u8,
    pub memory_mib: u32,
    pub network: PreparedNetwork,
    pub private_http_service: Option<PreparedPrivateHttpService>,
    pub runtime_git_bridge: Option<PreparedRuntimeGitBridge>,
    pub command: PreparedCommand,
    pub runtime_authority: Option<PreparedRuntimeAuthority>,
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedPrivateHttpService {
    pub loopback_port: u16,
    pub max_connections: u32,
    pub connect_timeout_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PreparedRuntimeGitBridge {
    pub repository_id: Uuid,
    pub loopback_port: u16,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PreparedRuntimeAuthority {
    pub session_id: Uuid,
    pub generation: u64,
    pub credential: [u8; RUNTIME_AUTHORITY_CREDENTIAL_BYTES],
    pub runtime_git_credential: Option<[u8; vm_trait::RUNTIME_GIT_CREDENTIAL_BYTES]>,
}

impl std::fmt::Debug for PreparedRuntimeAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedRuntimeAuthority")
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

impl Drop for PreparedRuntimeAuthority {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PreparedRoot {
    Directory { path: PathBuf },
    RawDisk { path: PathBuf, read_only: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedDisk {
    pub id: String,
    pub path: PathBuf,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedMount {
    pub tag: String,
    pub host_path: PathBuf,
    pub guest_path: PathBuf,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PreparedNetwork {
    Disabled,
    BrokerOnly,
    UserMode { ingress: Vec<PreparedForward> },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PreparedForward {
    pub bind_addr: IpAddr,
    pub host_port: u16,
    pub guest_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedCommand {
    pub program: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub working_dir: Option<PathBuf>,
}
