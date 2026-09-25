use std::{
    io::{self, Read, Write},
    net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpStream},
    os::unix::fs::PermissionsExt,
};

use super::{
    routing::bad_request,
    types::{
        AuthorityIsolationProof, BROKER_SOCKET_PATH, CONTROL_PARAMETERS_PATH, EgressIsolationProof,
        ISOLATION_PROBE_TIMEOUT, IsolationPorts, IsolationProof, MAX_ISOLATION_RESPONSE_BYTES,
        MountIsolationProof, NetworkIsolationProof, RUNTIME_AUTHORITY_ENV, RUNTIME_AUTHORITY_PATH,
        Response, SECRET_MOUNT_PATH, SERVICE_ADDRESS,
    },
};

pub(crate) fn isolation_response(target: &str) -> Response {
    let Some(ports) = parse_isolation_ports(target) else {
        return bad_request();
    };
    let proof = IsolationProof {
        network: NetworkIsolationProof {
            own_loopback_ok: probe_own_loopback(),
            admin_loopback_blocked: probe_blocked(loopback_address(ports.admin)),
            public_loopback_blocked: probe_blocked(loopback_address(ports.public)),
        },
        egress: EgressIsolationProof {
            metadata_blocked: probe_blocked(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
                80,
            )),
            test_net_blocked: probe_blocked(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
                80,
            )),
        },
        authority: AuthorityIsolationProof {
            runtime_authority_env_absent: std::env::var_os(RUNTIME_AUTHORITY_ENV).is_none(),
            runtime_authority_path_absent: fixed_path_absent(RUNTIME_AUTHORITY_PATH),
        },
        mounts: MountIsolationProof {
            broker_socket_absent: fixed_path_absent(BROKER_SOCKET_PATH),
            secret_mount_absent: fixed_path_absent(SECRET_MOUNT_PATH),
            control_surface_ok: control_surface_is_sealed(),
        },
    };
    let body = [
        String::from("\"schema_version\":1"),
        format!("\"own_loopback_ok\":{}", proof.network.own_loopback_ok),
        format!(
            "\"admin_loopback_blocked\":{}",
            proof.network.admin_loopback_blocked
        ),
        format!(
            "\"public_loopback_blocked\":{}",
            proof.network.public_loopback_blocked
        ),
        format!("\"metadata_blocked\":{}", proof.egress.metadata_blocked),
        format!("\"test_net_blocked\":{}", proof.egress.test_net_blocked),
        format!(
            "\"runtime_authority_env_absent\":{}",
            proof.authority.runtime_authority_env_absent
        ),
        format!(
            "\"runtime_authority_path_absent\":{}",
            proof.authority.runtime_authority_path_absent
        ),
        format!(
            "\"broker_socket_absent\":{}",
            proof.mounts.broker_socket_absent
        ),
        format!(
            "\"secret_mount_absent\":{}",
            proof.mounts.secret_mount_absent
        ),
        format!("\"control_surface_ok\":{}", proof.mounts.control_surface_ok),
    ]
    .join(",");
    let body = format!("{{{body}}}");
    Response {
        status: 200,
        content_type: "application/json",
        body: body.into_bytes(),
    }
}

pub(crate) fn parse_isolation_ports(target: &str) -> Option<IsolationPorts> {
    let (path, query) = target.split_once('?')?;
    if !matches!(path, "/service/isolation" | "/gateway/service/isolation") {
        return None;
    }
    let mut admin = None;
    let mut public = None;
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=')?;
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let port = value.parse::<u16>().ok()?;
        if port == 0 || port == SERVICE_ADDRESS.1 {
            return None;
        }
        match name {
            "admin_port" if admin.is_none() => admin = Some(port),
            "public_port" if public.is_none() => public = Some(port),
            _ => return None,
        }
    }
    let ports = IsolationPorts {
        admin: admin?,
        public: public?,
    };
    (ports.admin != ports.public).then_some(ports)
}

fn loopback_address(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

fn probe_blocked(address: SocketAddr) -> bool {
    TcpStream::connect_timeout(&address, ISOLATION_PROBE_TIMEOUT).is_err()
}

fn probe_own_loopback() -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(
        &loopback_address(SERVICE_ADDRESS.1),
        ISOLATION_PROBE_TIMEOUT,
    ) else {
        return false;
    };
    if stream
        .set_read_timeout(Some(ISOLATION_PROBE_TIMEOUT))
        .is_err()
        || stream
            .set_write_timeout(Some(ISOLATION_PROBE_TIMEOUT))
            .is_err()
    {
        return false;
    }
    if stream
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: service\r\n\r\n")
        .and_then(|()| stream.shutdown(Shutdown::Write))
        .is_err()
    {
        return false;
    }
    let mut response = Vec::with_capacity(MAX_ISOLATION_RESPONSE_BYTES);
    let Ok(read) = stream
        .take(u64::try_from(MAX_ISOLATION_RESPONSE_BYTES).expect("probe response bound"))
        .read_to_end(&mut response)
    else {
        return false;
    };
    if read == MAX_ISOLATION_RESPONSE_BYTES {
        return false;
    }
    response.starts_with(b"HTTP/1.1 200 OK\r\n")
        && response
            .windows(b"healthy".len())
            .any(|window| window == b"healthy")
}

fn fixed_path_absent(path: &str) -> bool {
    matches!(
        std::fs::symlink_metadata(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound
    )
}

fn control_surface_is_sealed() -> bool {
    let Ok(entries) = std::fs::read_dir("/run/hephaestus") else {
        return false;
    };
    let mut count = 0_usize;
    for entry in entries {
        let Ok(entry) = entry else {
            return false;
        };
        if entry.file_name() != "parameters.json" {
            return false;
        }
        count += 1;
        if count > 1 {
            return false;
        }
    }
    if count != 1 {
        return false;
    }
    let Ok(metadata) = std::fs::symlink_metadata(CONTROL_PARAMETERS_PATH) else {
        return false;
    };
    if !metadata.file_type().is_file()
        || metadata.permissions().mode() & 0o222 != 0
        || metadata.len() != 2
    {
        return false;
    }
    let Ok(mut file) = std::fs::File::open(CONTROL_PARAMETERS_PATH) else {
        return false;
    };
    let mut bytes = [0_u8; 2];
    file.read_exact(&mut bytes).is_ok() && bytes == *b"{}"
}
