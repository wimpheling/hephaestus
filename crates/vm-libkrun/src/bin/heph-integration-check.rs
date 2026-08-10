//! Guest-side hardware integration probe for the libkrun backend.

use brokered_egress_client::{BrokeredHttpsClient, WireBrokerRequest, WireBrokerStatus};
use runtime_types::RunId;
use rusqlite::Connection;
use serde::Deserialize;
use std::{
    fs,
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket},
    os::fd::FromRawFd,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
    time::Duration,
};

const BROKERED_E2E_RULE_ID: &str = "00000000-0000-0000-0000-000000000002";
const BROKERED_E2E_PLACEHOLDER: &str = "heph-placeholder:v1:00000000-0000-0000-0000-000000000002";
const BROKERED_E2E_CREDENTIAL_PATH: &str = "/run/hephaestus-secrets/.runtime-credential";
use vm_libkrun::protocol::{PrivateHttpRequestMessage, PrivateHttpResponseMessage};
use vm_trait::RUNTIME_AUTHORITY_CREDENTIAL_BYTES;

fn main() {
    if let Err(error) = run() {
        let _write_result = writeln!(io::stderr(), "integration-check: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    match std::env::args().nth(1).as_deref() {
        Some("--private-http-handler") => return private_http_handler().map_err(Into::into),
        Some("--private-http-brokered-header") => {
            return private_http_brokered_header_handler().map_err(Into::into);
        }
        Some("--serve-http") => return serve_http().map_err(Into::into),
        Some("--expect-network-disabled") => return expect_network_disabled(),
        Some("--expect-broker-only") => return expect_broker_only(),
        Some("--brokered-https-e2e") => return brokered_https_e2e(),
        Some("--expect-mailbox") => return expect_mailbox(),
        Some("--ignore-cancellation") => return ignore_cancellation(),
        Some("--state-only") => {
            verify_disk()?;
            println!("sqlite=ok");
            if let Ok(marker) = std::env::var("HEPH_RELEASE_MARKER") {
                println!("release_marker={marker}");
            }
            if let Ok(milliseconds) = std::env::var("HEPH_STATE_HOLD_MS") {
                std::thread::sleep(Duration::from_millis(milliseconds.parse()?));
            }
            return Ok(());
        }
        Some("--state-rollback") => {
            verify_sqlite_rollback()?;
            println!("sqlite-rollback=ok");
            std::process::exit(23);
        }
        Some(argument) => {
            return Err(format!("unknown integration-check argument: {argument}").into());
        }
        None => {}
    }

    eprintln!("stderr=ok");
    verify_disk()?;
    println!("sqlite=ok");
    verify_mounts()?;
    println!("mounts=ok");
    if std::env::var("HEPH_EXPECT_SECRET_MOUNT").as_deref() == Ok("1") {
        verify_secrets()?;
        println!("secrets=ok");
    }
    let resolved = ("example.com", 80)
        .to_socket_addrs()?
        .next()
        .ok_or("DNS returned no addresses")?;
    println!("dns=ok");
    verify_tcp(resolved)?;
    println!("tcp=ok");
    verify_udp_dns()?;
    println!("udp=ok");
    Ok(())
}

fn expect_mailbox() -> Result<(), Box<dyn std::error::Error>> {
    let envelope: serde_json::Value =
        serde_json::from_slice(&fs::read("/run/hephaestus/mailbox-event.json")?)?;
    if envelope["schema_version"] != 1
        || envelope["method"] != "POST"
        || envelope["route"] != "/mailbox/libkrun-proof"
        || envelope["body_path"] != "/run/hephaestus/mailbox-body"
    {
        return Err("mailbox envelope is not the exact generic control contract".into());
    }
    if fs::read("/run/hephaestus/mailbox-body")? != b"real-libkrun-mailbox-body" {
        return Err("mailbox body was not delivered exactly".into());
    }
    println!("mailbox=ok");
    Ok(())
}

/// Minimal released-command fixture for the real one-request gateway ABI.
/// It deliberately has no network listener: the host can only reach it over
/// the authenticated private control channel via `heph-init`.
fn private_http_handler() -> io::Result<()> {
    let request: PrivateHttpRequestMessage =
        ciborium::from_reader(io::stdin().lock()).map_err(io::Error::other)?;
    if request.method != "POST" || request.path_and_query != "/gateway/proof?mode=real" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected private HTTP request",
        ));
    }
    if request.body != b"gateway-real-vm-request" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "private HTTP body was not delivered exactly",
        ));
    }
    let response = PrivateHttpResponseMessage {
        status: 201,
        headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
        body: b"gateway-real-vm-response".to_vec(),
    };
    ciborium::into_writer(&response, io::stdout().lock()).map_err(io::Error::other)
}

/// Confirms the daemon's host-only gateway resolver replaced a real inbound
/// credential with its non-secret immutable placeholder before VM delivery.
fn private_http_brokered_header_handler() -> io::Result<()> {
    let request: PrivateHttpRequestMessage =
        ciborium::from_reader(io::stdin().lock()).map_err(io::Error::other)?;
    if request.method != "POST" || request.path_and_query != "/gateway/brokered?mode=real" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected brokered gateway request",
        ));
    }
    let placeholder = request
        .headers
        .iter()
        .find(|(name, _)| name == "x-webhook-secret")
        .map(|(_, value)| value.as_str());
    if !placeholder.is_some_and(|value| value.starts_with("heph-placeholder:v1:")) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "gateway secret was not replaced by its placeholder",
        ));
    }
    let response = PrivateHttpResponseMessage {
        status: 201,
        headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
        body: b"gateway-brokered-header-ok".to_vec(),
    };
    ciborium::into_writer(&response, io::stdout().lock()).map_err(io::Error::other)
}

fn verify_disk() -> Result<(), Box<dyn std::error::Error>> {
    let database = Path::new("/var/lib/hephaestus/state.db");
    let connection = Connection::open(database)?;
    let journal_mode: String = connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err("SQLite state database is not in WAL mode".into());
    }
    let table_count: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'probe'",
        [],
        |row| row.get(0),
    )?;
    let previous_rows: i64 = if table_count == 0 {
        0
    } else {
        connection.query_row("SELECT count(*) FROM probe", [], |row| row.get(0))?
    };
    println!("sqlite_previous={previous_rows}");
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS probe(value TEXT);
         INSERT INTO probe VALUES('ok');",
    )?;
    let value: String = connection.query_row(
        "SELECT value FROM probe ORDER BY rowid DESC LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    if value != "ok" {
        return Err("SQLite readback failed".into());
    }
    drop(connection);
    Ok(())
}

fn verify_sqlite_rollback() -> Result<(), Box<dyn std::error::Error>> {
    let database = Path::new("/var/lib/hephaestus/state.db");
    let mut connection = Connection::open(database)?;
    let before: i64 = connection.query_row("SELECT count(*) FROM probe", [], |row| row.get(0))?;
    let transaction = connection.transaction()?;
    transaction.execute("INSERT INTO probe VALUES('must-rollback')", [])?;
    transaction.rollback()?;
    let after: i64 = connection.query_row("SELECT count(*) FROM probe", [], |row| row.get(0))?;
    if after != before {
        return Err("agent-owned SQLite rollback retained a mutation".into());
    }
    Ok(())
}

fn verify_mounts() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::current_dir()? != Path::new("/workspace") {
        return Err("requested working directory was not applied".into());
    }
    if fs::read_to_string("/repository/integration-marker")?.trim() != "repository" {
        return Err("read-only repository marker is invalid".into());
    }
    match fs::write("/repository/write-must-fail", "invalid") {
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
            ) => {}
        Err(error) => return Err(format!("unexpected read-only mount error: {error}").into()),
        Ok(()) => return Err("read-only repository accepted a write".into()),
    }
    let workspace_marker = Path::new("/workspace/integration-marker");
    fs::write(workspace_marker, "workspace")?;
    if fs::read_to_string(workspace_marker)? != "workspace" {
        return Err("writable workspace readback failed".into());
    }
    Ok(())
}

fn verify_secrets() -> Result<(), Box<dyn std::error::Error>> {
    const SENTINEL: &str = "libkrun-secret-sentinel-8a4c";
    let directory = Path::new("/run/hephaestus-secrets");
    let secret = directory.join("model");
    let directory_metadata = fs::symlink_metadata(directory)?;
    let metadata = fs::symlink_metadata(&secret)?;
    if !directory_metadata.is_dir() || !metadata.is_file() {
        return Err("raw secret mount contains an unexpected object".into());
    }
    if directory_metadata.permissions().mode() & 0o777 != 0o500
        || metadata.permissions().mode() & 0o777 != 0o400
    {
        return Err("raw secret mount permissions are unsafe".into());
    }
    if metadata.uid() != 10_001 || metadata.gid() != 10_001 {
        return Err("raw secret ownership does not match the guest agent".into());
    }
    if fs::read_to_string(&secret)? != SENTINEL {
        return Err("raw secret contents do not match the exact slot".into());
    }
    for path in [&secret, &directory.join("write-must-fail")] {
        match fs::write(path, "invalid") {
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
                ) => {}
            Err(error) => {
                return Err(format!("unexpected secret mount write error: {error}").into());
            }
            Ok(()) => return Err("read-only raw secret mount accepted a write".into()),
        }
    }
    for proc_file in ["/proc/self/environ", "/proc/self/cmdline"] {
        if fs::read(proc_file)?
            .windows(SENTINEL.len())
            .any(|window| window == SENTINEL.as_bytes())
        {
            return Err("raw secret leaked into process metadata".into());
        }
    }
    Ok(())
}

fn serve_http() -> io::Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", 8080))?;
    println!("http=ready");
    let (mut stream, _) = listener.accept()?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut request = [0_u8; 1024];
    let read = stream.read(&mut request)?;
    if read == 0 || !request[..read].starts_with(b"GET / HTTP/1.") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid HTTP request",
        ));
    }
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")?;
    stream.flush()?;
    stream.shutdown(Shutdown::Write)?;
    // Keep the VMM alive briefly so passt can drain its socket before the
    // command's final exit tears down virtio-net.
    std::thread::sleep(Duration::from_millis(100));
    Ok(())
}

fn expect_network_disabled() -> Result<(), Box<dyn std::error::Error>> {
    let address: SocketAddr = "1.1.1.1:80".parse()?;
    if TcpStream::connect_timeout(&address, Duration::from_millis(500)).is_ok() {
        return Err("disabled network unexpectedly allowed outbound TCP".into());
    }
    println!("network-disabled=ok");
    Ok(())
}

/// Proves the broker-only VM contract retains its dedicated vsock endpoint
/// while denying ordinary TCP egress.
fn expect_broker_only() -> Result<(), Box<dyn std::error::Error>> {
    expect_network_disabled()?;
    let mut authority = read_runtime_authority()?;
    let request_body = serde_json::json!({
        "rule_id": "00000000-0000-0000-0000-000000000002",
        "method": "get",
        "path_and_query": "/v1/probe",
        "headers": [],
        "body": []
    });
    let mut request = WireBrokerRequest {
        credential: std::mem::take(&mut authority.credential),
        run_id: RunId::from_uuid(authority.session_id),
        slot: String::from("model"),
        destination: String::from("api.example.test"),
        operation: String::from("https_v1"),
        body: serde_json::to_vec(&request_body)?,
    };
    let response = BrokeredHttpsClient::new(vsock_broker_stream()?).call(&request)?;
    request.credential.fill(0);
    if response.status != WireBrokerStatus::Succeeded || response.body != b"ok" {
        return Err("unexpected broker response".into());
    }
    println!("broker-vsock=ok");
    Ok(())
}

/// Exercises the real secret-runtime credential issued into the protected
/// brokered mount by the daemon's `PostgreSQL` resolver. The provider secret is
/// represented only by the stable placeholder in this released guest.
fn brokered_https_e2e() -> Result<(), Box<dyn std::error::Error>> {
    expect_network_disabled()?;
    let authority = read_runtime_authority()?;
    let mut credential = fs::read(BROKERED_E2E_CREDENTIAL_PATH)?;
    if credential.len() != RUNTIME_AUTHORITY_CREDENTIAL_BYTES {
        return Err("brokered runtime credential has an invalid length".into());
    }
    let request_body = serde_json::json!({
        "rule_id": BROKERED_E2E_RULE_ID,
        "method": "get",
        "path_and_query": "/v1/probe",
        "headers": [{
            "name": "authorization",
            "value": format!("Bearer {BROKERED_E2E_PLACEHOLDER}")
        }],
        "body": []
    });
    let mut request = WireBrokerRequest {
        credential: std::mem::take(&mut credential),
        // The daemon deliberately derives the generic runtime session ID from
        // the run ID, so this guest-visible non-secret identifier safely
        // names the exact secret-runtime lease without exposing its bearer.
        run_id: RunId::from_uuid(authority.session_id),
        slot: String::from("model"),
        destination: String::from("api.example.test"),
        operation: String::from("https_v1"),
        body: serde_json::to_vec(&request_body)?,
    };
    let response = BrokeredHttpsClient::new(vsock_broker_stream()?).call(&request)?;
    request.credential.fill(0);
    if response.status != WireBrokerStatus::Succeeded || response.body != b"brokered-e2e-ok" {
        return Err("unexpected brokered HTTPS response".into());
    }
    println!("brokered-https-e2e=ok");
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuestRuntimeAuthority {
    session_id: uuid::Uuid,
    generation: u64,
    credential_hex: String,
}

struct RuntimeAuthorityCredential {
    session_id: uuid::Uuid,
    credential: Vec<u8>,
}

fn read_runtime_authority() -> Result<RuntimeAuthorityCredential, Box<dyn std::error::Error>> {
    let authority: GuestRuntimeAuthority = serde_json::from_slice(&fs::read(
        vm_libkrun::protocol::GUEST_RUNTIME_AUTHORITY_PATH,
    )?)?;
    if authority.generation == 0 {
        return Err("runtime authority generation is invalid".into());
    }
    if authority.credential_hex.len() != 64 {
        return Err("runtime authority credential has an invalid length".into());
    }
    let mut credential = Vec::with_capacity(32);
    for offset in (0..authority.credential_hex.len()).step_by(2) {
        credential.push(u8::from_str_radix(
            &authority.credential_hex[offset..offset + 2],
            16,
        )?);
    }
    Ok(RuntimeAuthorityCredential {
        session_id: authority.session_id,
        credential,
    })
}

#[allow(unsafe_code)] // AF_VSOCK is exposed only through libc's raw socket ABI.
fn vsock_broker_stream() -> io::Result<fs::File> {
    #[repr(C)]
    struct SockAddrVm {
        family: libc::sa_family_t,
        reserved: u16,
        port: u32,
        cid: u32,
        zero: [u8; 4],
    }

    // SAFETY: this guest fixture creates one AF_VSOCK stream and immediately
    // transfers its owned file descriptor to File after a checked connect.
    let descriptor = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM, 0) };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    let family = libc::sa_family_t::try_from(libc::AF_VSOCK)
        .map_err(|_| io::Error::other("AF_VSOCK address family is invalid"))?;
    let address = SockAddrVm {
        family,
        reserved: 0,
        port: vm_libkrun::protocol::SECRET_BROKER_VSOCK_PORT,
        cid: 2, // VMADDR_CID_HOST is the stable host CID for libkrun guests.
        zero: [0; 4],
    };
    let length = libc::socklen_t::try_from(std::mem::size_of::<SockAddrVm>())
        .map_err(|_| io::Error::other("vsock address is oversized"))?;
    // SAFETY: address is a fully initialized repr(C) sockaddr_vm and its
    // pointer remains valid for the duration of this synchronous syscall.
    let connected = unsafe {
        libc::connect(
            descriptor,
            std::ptr::from_ref(&address).cast::<libc::sockaddr>(),
            length,
        )
    };
    if connected != 0 {
        // SAFETY: descriptor is still exclusively owned after a failed connect.
        let _closed = unsafe { libc::close(descriptor) };
        return Err(io::Error::last_os_error());
    }
    // SAFETY: connect succeeded and ownership transfers exactly once into File.
    Ok(unsafe { fs::File::from_raw_fd(descriptor) })
}

#[allow(unsafe_code)]
fn ignore_cancellation() -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: installing SIG_IGN for SIGTERM changes only this isolated guest
    // test process and does not dereference memory.
    let previous = unsafe { libc::signal(libc::SIGTERM, libc::SIG_IGN) };
    if previous == libc::SIG_ERR {
        return Err(io::Error::last_os_error().into());
    }
    println!("ignore-cancellation=ready");
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

fn verify_tcp(address: SocketAddr) -> io::Result<()> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(10))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.write_all(b"HEAD / HTTP/1.0\r\nHost: example.com\r\n\r\n")?;
    let mut response = [0_u8; 16];
    let read = stream.read(&mut response)?;
    if read == 0 {
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "TCP peer returned no response",
        ))
    } else {
        Ok(())
    }
}

fn verify_udp_dns() -> Result<(), Box<dyn std::error::Error>> {
    let resolver = resolver_address()?;
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    let query = dns_query();
    socket.send_to(&query, resolver)?;
    let mut response = [0_u8; 512];
    let (length, _) = socket.recv_from(&mut response)?;
    if length < 12 || response[..2] != query[..2] || response[3] & 0x0f != 0 {
        return Err("invalid UDP DNS response".into());
    }
    Ok(())
}

fn resolver_address() -> Result<SocketAddr, Box<dyn std::error::Error>> {
    let resolv_conf = fs::read_to_string("/etc/resolv.conf")?;
    let address = resolv_conf
        .lines()
        .find_map(|line| line.strip_prefix("nameserver "))
        .ok_or("no DNS resolver is configured")?;
    Ok(format!("{}:53", address.trim()).parse()?)
}

fn dns_query() -> Vec<u8> {
    let mut query = vec![
        0x48, 0x50, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    for label in ["example", "com"] {
        query.push(u8::try_from(label.len()).expect("DNS label length fits u8"));
        query.extend_from_slice(label.as_bytes());
    }
    query.extend_from_slice(&[0, 0, 1, 0, 1]);
    query
}
