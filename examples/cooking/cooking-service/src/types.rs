use std::{
    io,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(crate) const SERVICE_ADDRESS: (&str, u16) = ("127.0.0.1", 8080);
pub(crate) const WORKER_COUNT: usize = 4;
pub(crate) const QUEUE_CAPACITY: usize = 8;
pub(crate) const MAX_HEADER_BYTES: usize = 8 * 1024;
pub(crate) const MAX_HEADER_COUNT: usize = 32;
pub(crate) const MAX_HEADER_LINE_BYTES: usize = 4 * 1024;
pub(crate) const IO_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const ISOLATION_PROBE_TIMEOUT: Duration = Duration::from_millis(150);
pub(crate) const MAX_ISOLATION_RESPONSE_BYTES: usize = 256;
pub(crate) const RUNTIME_AUTHORITY_ENV: &str = "HEPH_RUNTIME_AUTHORITY_PATH";
pub(crate) const RUNTIME_AUTHORITY_PATH: &str = "/run/hephaestus-authority/session.json";
pub(crate) const BROKER_SOCKET_PATH: &str = "/run/hephaestus/broker.sock";
pub(crate) const SECRET_MOUNT_PATH: &str = "/run/hephaestus-secrets";
pub(crate) const CONTROL_PARAMETERS_PATH: &str = "/run/hephaestus/parameters.json";
pub(crate) const EXPECTED_GATEWAY_AUTHORITY: &str = "gateway.golden.invalid";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StartupIdentity {
    pub(crate) pid: u32,
    pub(crate) value: String,
}

impl StartupIdentity {
    pub(crate) fn new() -> io::Result<Self> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let pid = std::process::id();
        Ok(Self {
            pid,
            value: format!("{pid}-{timestamp}"),
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) content_type: &'static str,
    pub(crate) body: Vec<u8>,
}

// Each field is a separate redacted header-presence assertion in the fixture.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RequestMetadata {
    pub(crate) host_matches_expected: bool,
    pub(crate) forwarded_present: bool,
    pub(crate) x_forwarded_for_present: bool,
    pub(crate) x_forwarded_host_present: bool,
    pub(crate) x_forwarded_proto_present: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct IsolationPorts {
    pub(crate) admin: u16,
    pub(crate) public: u16,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct IsolationProof {
    pub(crate) network: NetworkIsolationProof,
    pub(crate) egress: EgressIsolationProof,
    pub(crate) authority: AuthorityIsolationProof,
    pub(crate) mounts: MountIsolationProof,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct NetworkIsolationProof {
    pub(crate) own_loopback_ok: bool,
    pub(crate) admin_loopback_blocked: bool,
    pub(crate) public_loopback_blocked: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EgressIsolationProof {
    pub(crate) metadata_blocked: bool,
    pub(crate) test_net_blocked: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AuthorityIsolationProof {
    pub(crate) runtime_authority_env_absent: bool,
    pub(crate) runtime_authority_path_absent: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MountIsolationProof {
    pub(crate) broker_socket_absent: bool,
    pub(crate) secret_mount_absent: bool,
    pub(crate) control_surface_ok: bool,
}
