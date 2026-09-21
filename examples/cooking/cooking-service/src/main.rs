//! Dependency-free long-lived HTTP service fixture for the persistent gateway.
//!
//! The process deliberately owns no Hephaestus authority. It listens only on
//! the declared guest-loopback port and serves bounded ordinary HTTP responses.

use std::{
    io::{self, ErrorKind, Read, Write},
    net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const SERVICE_ADDRESS: (&str, u16) = ("127.0.0.1", 8080);
const WORKER_COUNT: usize = 4;
const QUEUE_CAPACITY: usize = 8;
const MAX_HEADER_BYTES: usize = 8 * 1024;
const MAX_HEADER_COUNT: usize = 32;
const MAX_HEADER_LINE_BYTES: usize = 4 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const ISOLATION_PROBE_TIMEOUT: Duration = Duration::from_millis(150);
const MAX_ISOLATION_RESPONSE_BYTES: usize = 256;
const RUNTIME_AUTHORITY_ENV: &str = "HEPH_RUNTIME_AUTHORITY_PATH";
const RUNTIME_AUTHORITY_PATH: &str = "/run/hephaestus-authority/session.json";
const BROKER_SOCKET_PATH: &str = "/run/hephaestus/broker.sock";
const SECRET_MOUNT_PATH: &str = "/run/hephaestus-secrets";
const CONTROL_PARAMETERS_PATH: &str = "/run/hephaestus/parameters.json";
const EXPECTED_GATEWAY_AUTHORITY: &str = "gateway.golden.invalid";

#[derive(Clone, Debug, PartialEq, Eq)]
struct StartupIdentity {
    pid: u32,
    value: String,
}

impl StartupIdentity {
    fn new() -> io::Result<Self> {
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
struct Response {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

// Each field is a separate redacted header-presence assertion in the fixture.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default, PartialEq, Eq)]
struct RequestMetadata {
    host_matches_expected: bool,
    forwarded_present: bool,
    x_forwarded_for_present: bool,
    x_forwarded_host_present: bool,
    x_forwarded_proto_present: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct IsolationPorts {
    admin: u16,
    public: u16,
}

#[derive(Debug, PartialEq, Eq)]
struct IsolationProof {
    network: NetworkIsolationProof,
    egress: EgressIsolationProof,
    authority: AuthorityIsolationProof,
    mounts: MountIsolationProof,
}

#[derive(Debug, PartialEq, Eq)]
struct NetworkIsolationProof {
    own_loopback_ok: bool,
    admin_loopback_blocked: bool,
    public_loopback_blocked: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct EgressIsolationProof {
    metadata_blocked: bool,
    test_net_blocked: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct AuthorityIsolationProof {
    runtime_authority_env_absent: bool,
    runtime_authority_path_absent: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct MountIsolationProof {
    broker_socket_absent: bool,
    secret_mount_absent: bool,
    control_surface_ok: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(SERVICE_ADDRESS)?;
    let identity = StartupIdentity::new()?;
    println!(
        "cooking-service ready on {}:{} pid={}",
        SERVICE_ADDRESS.0, SERVICE_ADDRESS.1, identity.pid
    );
    run(&listener, identity)?;
    Ok(())
}

fn run(listener: &TcpListener, identity: StartupIdentity) -> io::Result<()> {
    let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
    let receiver = Arc::new(Mutex::new(receiver));
    let identity = Arc::new(identity);
    for _ in 0..WORKER_COUNT {
        let receiver = Arc::clone(&receiver);
        let identity = Arc::clone(&identity);
        thread::Builder::new()
            .name(String::from("cooking-service-worker"))
            .spawn(move || {
                loop {
                    let connection = receiver
                        .lock()
                        .expect("service worker queue mutex is not poisoned")
                        .recv();
                    let Ok(stream) = connection else {
                        break;
                    };
                    let _ = serve_connection(stream, &identity);
                }
            })
            .map_err(io::Error::other)?;
    }

    loop {
        let (stream, _) = listener.accept()?;
        if sender.send(stream).is_err() {
            return Err(io::Error::new(
                ErrorKind::BrokenPipe,
                "service worker queue closed",
            ));
        }
    }
}

fn serve_connection(mut stream: TcpStream, identity: &StartupIdentity) -> io::Result<()> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let response = match read_request(&mut stream) {
        Ok(request) => response_for(request.as_slice(), identity),
        Err(_) => Response {
            status: 400,
            content_type: "text/plain; charset=utf-8",
            body: b"bad request".to_vec(),
        },
    };
    write_response(&mut stream, &response)?;
    stream.shutdown(Shutdown::Write)
}

fn read_request(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    read_request_with_budget(stream, IO_TIMEOUT)
}

fn read_request_with_budget(stream: &mut TcpStream, budget: Duration) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + budget;
    let mut request = Vec::with_capacity(1024);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                ErrorKind::TimedOut,
                "request header deadline elapsed",
            ));
        }
        stream.set_read_timeout(Some(remaining))?;
        let mut chunk = [0_u8; 1024];
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(io::Error::new(
                ErrorKind::UnexpectedEof,
                "request ended before headers",
            ));
        }
        request.extend_from_slice(&chunk[..read]);
        if request.len() > MAX_HEADER_BYTES {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "request headers exceed the service limit",
            ));
        }
        if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            let end = end + 4;
            if request.len() != end {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    "request body is not supported",
                ));
            }
            request.truncate(end);
            return Ok(request);
        }
    }
}

fn response_for(request: &[u8], identity: &StartupIdentity) -> Response {
    let Ok(request) = std::str::from_utf8(request) else {
        return bad_request();
    };
    let mut lines = request.split("\r\n");
    let Some(request_line) = lines.next() else {
        return bad_request();
    };
    let mut parts = request_line.split_ascii_whitespace();
    let (Some(method), Some(target), Some(version)) = (parts.next(), parts.next(), parts.next())
    else {
        return bad_request();
    };
    if method != "GET"
        || !matches!(version, "HTTP/1.0" | "HTTP/1.1")
        || parts.next().is_some()
        || !target.starts_with('/')
    {
        return bad_request();
    }

    let mut header_count = 0_usize;
    let mut metadata = RequestMetadata::default();
    let mut host = None;
    for line in lines {
        if line.is_empty() {
            break;
        }
        if line.len() > MAX_HEADER_LINE_BYTES {
            return bad_request();
        }
        let Some((name, value)) = line.split_once(':') else {
            return bad_request();
        };
        if name.is_empty()
            || name
                .bytes()
                .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
        {
            return bad_request();
        }
        header_count += 1;
        if header_count > MAX_HEADER_COUNT {
            return bad_request();
        }
        if name.eq_ignore_ascii_case("upgrade")
            || name.eq_ignore_ascii_case("transfer-encoding")
            || (name.eq_ignore_ascii_case("connection")
                && value
                    .split(',')
                    .any(|token| token.trim().eq_ignore_ascii_case("upgrade")))
        {
            return bad_request();
        }
        if name.eq_ignore_ascii_case("content-length") && value.trim() != "0" {
            return bad_request();
        }
        if name.eq_ignore_ascii_case("host") {
            if host.replace(value.trim().to_owned()).is_some() {
                return bad_request();
            }
        } else if name.eq_ignore_ascii_case("forwarded") {
            metadata.forwarded_present = true;
        } else if name.eq_ignore_ascii_case("x-forwarded-for") {
            metadata.x_forwarded_for_present = true;
        } else if name.eq_ignore_ascii_case("x-forwarded-host") {
            metadata.x_forwarded_host_present = true;
        } else if name.eq_ignore_ascii_case("x-forwarded-proto") {
            metadata.x_forwarded_proto_present = true;
        }
    }
    metadata.host_matches_expected = host.as_deref() == Some(EXPECTED_GATEWAY_AUTHORITY);
    let path = target.split_once('?').map_or(target, |(path, _)| path);
    match path {
        "/" | "/service" | "/gateway/service" => text_response(b"cooking service"),
        "/readyz" => text_response(b"ready"),
        "/healthz" => text_response(b"healthy"),
        "/identity" | "/service/identity" | "/gateway/service/identity" => {
            identity_response(identity)
        }
        "/service/isolation" | "/gateway/service/isolation" => isolation_response(target),
        "/service/metadata" | "/gateway/service/metadata" => metadata_response(&metadata),
        _ => Response {
            status: 404,
            content_type: "text/plain; charset=utf-8",
            body: b"not found".to_vec(),
        },
    }
}

fn metadata_response(metadata: &RequestMetadata) -> Response {
    let body = format!(
        concat!(
            "{{\"host_matches_expected\":{},",
            "\"forwarded_present\":{},",
            "\"x_forwarded_for_present\":{},",
            "\"x_forwarded_host_present\":{},",
            "\"x_forwarded_proto_present\":{}}}"
        ),
        metadata.host_matches_expected,
        metadata.forwarded_present,
        metadata.x_forwarded_for_present,
        metadata.x_forwarded_host_present,
        metadata.x_forwarded_proto_present,
    );
    Response {
        status: 200,
        content_type: "application/json",
        body: body.into_bytes(),
    }
}

fn isolation_response(target: &str) -> Response {
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

fn parse_isolation_ports(target: &str) -> Option<IsolationPorts> {
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

fn write_response(stream: &mut TcpStream, response: &Response) -> io::Result<()> {
    let reason = match response.status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => return Err(io::Error::new(ErrorKind::InvalidInput, "invalid status")),
    };
    write!(
        stream,
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        response.content_type,
        response.body.len()
    )?;
    stream.write_all(&response.body)
}

fn text_response(body: &'static [u8]) -> Response {
    Response {
        status: 200,
        content_type: "text/plain; charset=utf-8",
        body: body.to_vec(),
    }
}

fn identity_response(identity: &StartupIdentity) -> Response {
    Response {
        status: 200,
        content_type: "application/json",
        body: format!(
            r#"{{"pid":{},"startup_id":"{}"}}"#,
            identity.pid, identity.value
        )
        .into_bytes(),
    }
}

fn bad_request() -> Response {
    Response {
        status: 400,
        content_type: "text/plain; charset=utf-8",
        body: b"bad request".to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn test_identity() -> StartupIdentity {
        StartupIdentity {
            pid: 7,
            value: String::from("7-test-startup"),
        }
    }

    fn round_trip(request: &[u8], identity: &StartupIdentity) -> Vec<u8> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("test listener");
        let address = listener.local_addr().expect("test address");
        let identity = identity.clone();
        let worker = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("test connection");
            serve_connection(stream, &identity).expect("serve test request");
        });
        let mut client = TcpStream::connect(address).expect("test client");
        client.write_all(request).expect("write test request");
        client
            .shutdown(Shutdown::Write)
            .expect("finish test request");
        let mut response = Vec::new();
        client
            .read_to_end(&mut response)
            .expect("read test response");
        worker.join().expect("test worker");
        response
    }

    #[test]
    fn repeated_identity_requests_keep_one_process_identity() {
        let identity = test_identity();
        for path in ["/service/identity", "/gateway/service/identity"] {
            let request = format!("GET {path} HTTP/1.1\r\nHost: service\r\n\r\n");
            let first = round_trip(request.as_bytes(), &identity);
            let second = round_trip(request.as_bytes(), &identity);
            assert_eq!(first, second);
            assert!(
                first
                    .windows(b"7-test-startup".len())
                    .any(|window| { window == b"7-test-startup" })
            );
        }
    }

    #[test]
    fn probes_and_public_routes_have_bounded_http_responses() {
        let identity = test_identity();
        for (path, expected) in [
            ("/readyz", b"ready".as_slice()),
            ("/healthz", b"healthy".as_slice()),
            ("/", b"cooking service".as_slice()),
            ("/service", b"cooking service".as_slice()),
            ("/gateway/service", b"cooking service".as_slice()),
            (
                "/gateway/service/metadata",
                b"{\"host_matches_expected\":false,\"forwarded_present\":false,\"x_forwarded_for_present\":false,\"x_forwarded_host_present\":false,\"x_forwarded_proto_present\":false}".as_slice(),
            ),
            (
                "/gateway/service/identity",
                b"\"startup_id\":\"7-test-startup\"".as_slice(),
            ),
        ] {
            let request = format!("GET {path} HTTP/1.1\r\nHost: service\r\n\r\n");
            let response = round_trip(request.as_bytes(), &identity);
            assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
            if path.ends_with("identity") {
                assert!(
                    response
                        .windows(expected.len())
                        .any(|window| window == expected)
                );
            } else {
                assert!(response.ends_with(expected));
            }
            assert!(
                response
                    .windows(b"Connection: close".len())
                    .any(|window| { window.eq_ignore_ascii_case(b"Connection: close") })
            );
        }
    }

    #[test]
    fn isolation_query_accepts_only_distinct_non_service_ports() {
        assert_eq!(
            parse_isolation_ports("/gateway/service/isolation?admin_port=2019&public_port=18080"),
            Some(IsolationPorts {
                admin: 2019,
                public: 18080,
            })
        );
        for query in [
            "/service/isolation?admin_port=8080&public_port=18080",
            "/service/isolation?admin_port=2019&public_port=2019",
            "/service/isolation?admin_port=2019&public_port=18080&host=127.0.0.1",
            "/service/isolation?admin_port=not-a-port&public_port=18080",
            "/service/isolation?admin_port=+2019&public_port=18080",
            "/service/isolation?admin_port=65536&public_port=18080",
            "/service/isolation?admin_port=0&public_port=18080",
            "/service/isolation?admin_port=2019",
        ] {
            assert_eq!(parse_isolation_ports(query), None, "query: {query}");
        }
    }

    #[test]
    fn isolation_routes_reject_arbitrary_destinations() {
        let identity = test_identity();
        let response = response_for(
            b"GET /gateway/service/isolation?admin_port=2019&public_port=18080&host=169.254.169.254 HTTP/1.1\r\nHost: service\r\n\r\n",
            &identity,
        );
        assert_eq!(response.status, 400);
    }

    #[test]
    fn metadata_reports_only_case_insensitive_forwarding_presence() {
        let identity = test_identity();
        let response = response_for(
            b"GET /service/metadata HTTP/1.1\r\nHoSt: gateway.golden.invalid\r\nfOrWaRdEd: for=attacker\r\nX-FORWARDED-FOR: 192.0.2.7\r\nX-Forwarded-Host: attacker.invalid\r\nX-forwarded-PROTO: http\r\nX-Secret: do-not-report\r\n\r\n",
            &identity,
        );
        assert_eq!(response.status, 200);
        let body = String::from_utf8(response.body).expect("metadata JSON UTF-8");
        assert!(body.contains("\"host_matches_expected\":true"));
        assert!(body.contains("\"forwarded_present\":true"));
        assert!(body.contains("\"x_forwarded_for_present\":true"));
        assert!(body.contains("\"x_forwarded_host_present\":true"));
        assert!(body.contains("\"x_forwarded_proto_present\":true"));
        assert!(!body.contains("attacker"));
        assert!(!body.contains("do-not-report"));
    }

    #[test]
    fn metadata_rejects_duplicate_host_headers() {
        let identity = test_identity();
        let response = response_for(
            b"GET /gateway/service/metadata HTTP/1.1\r\nHost: gateway.golden.invalid\r\nhOsT: attacker.invalid\r\n\r\n",
            &identity,
        );
        assert_eq!(response.status, 400);
    }

    #[test]
    fn rejects_body_and_protocol_upgrade_requests() {
        let identity = test_identity();
        for request in [
            b"GET / HTTP/1.1\r\nHost: service\r\nContent-Length: 1\r\n\r\nx".as_slice(),
            b"GET / HTTP/1.1\r\nHost: service\r\nUpgrade: websocket\r\n\r\n".as_slice(),
        ] {
            let response = round_trip(request, &identity);
            assert!(response.starts_with(b"HTTP/1.1 400 Bad Request\r\n"));
        }
    }

    #[test]
    fn trickled_headers_share_one_absolute_read_deadline() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("test listener");
        let address = listener.local_addr().expect("test address");
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test connection");
            read_request_with_budget(&mut stream, Duration::from_millis(100))
        });
        let mut client = TcpStream::connect(address).expect("test client");
        let started = Instant::now();
        client
            .write_all(b"GET / HTTP/1.1\r\n")
            .expect("write first header chunk");
        thread::sleep(Duration::from_millis(70));
        client
            .write_all(b"Host: service\r\n")
            .expect("write second header chunk");
        let result = worker.join().expect("test worker");
        assert!(matches!(
            result.expect_err("trickled request must time out").kind(),
            ErrorKind::TimedOut | ErrorKind::WouldBlock
        ));
        assert!(started.elapsed() < Duration::from_millis(150));
    }
}
