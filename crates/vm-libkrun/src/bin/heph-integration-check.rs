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
    process,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

const BROKERED_E2E_RULE_ID: &str = "00000000-0000-0000-0000-000000000002";
const BROKERED_E2E_PLACEHOLDER: &str = "heph-placeholder:v1:00000000-0000-0000-0000-000000000002";
const BROKERED_E2E_CREDENTIAL_PATH: &str = "/run/hephaestus-secrets/.runtime-credential";
const SERVICE_DEFAULT_PORT: u16 = 8080;
const SERVICE_MAX_REQUESTS: usize = 128;
const SERVICE_MAX_CONNECTIONS: usize = 4;
const SERVICE_MAX_REQUEST_BYTES: usize = 16 * 1024;
const SERVICE_MAX_HEADER_COUNT: usize = 32;
const SERVICE_MAX_HEADER_LINE_BYTES: usize = 4 * 1024;
const SERVICE_MAX_BODY_BYTES: usize = 16 * 1024;
const SERVICE_MAX_DELAY_MS: u64 = 5_000;
const SERVICE_IO_TIMEOUT: Duration = Duration::from_secs(5);
const SERVICE_CRASH_EXIT_CODE: i32 = 42;
const SERVICE_ISOLATION_CHECK_ENV: &str = "HEPH_SERVICE_ISOLATION_CHECK";
const RUNTIME_AUTHORITY_ENV: &str = "HEPH_RUNTIME_AUTHORITY_PATH";
const RUNTIME_AUTHORITY_FILE: &str = "/run/hephaestus-authority/session.json";
use vm_libkrun::protocol::{
    PrivateHttpRequestMessage, PrivateHttpResponseMessage, PrivateMailboxPublicationMessage,
};
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
        Some("--private-http-brokered-mailbox") => {
            return private_http_brokered_mailbox_handler().map_err(Into::into);
        }
        Some("--serve-http") => return serve_http().map_err(Into::into),
        Some("--serve-service") => return serve_service().map_err(Into::into),
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
        mailbox_publication: None,
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
        mailbox_publication: None,
    };
    ciborium::into_writer(&response, io::stdout().lock()).map_err(io::Error::other)
}

/// Real-guest fixture for the bounded gateway-to-mailbox handoff.  The
/// mailbox name and producer identity deliberately never enter the guest:
/// this only selects the release-declared symbolic slot and supplies a stable
/// application-level idempotency key.
fn private_http_brokered_mailbox_handler() -> io::Result<()> {
    let request: PrivateHttpRequestMessage =
        ciborium::from_reader(io::stdin().lock()).map_err(io::Error::other)?;
    if request.method != "POST" || request.path_and_query != "/gateway/brokered?mode=real" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected brokered mailbox gateway request",
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
    let deduplication_key = match request.body.as_slice() {
        b"gateway-brokered-request-a" => "gateway-real-request-a",
        b"gateway-brokered-request-b" => "gateway-real-request-b",
        // This is accepted by the handler so the golden path can prove that
        // revocation stops an otherwise valid, fresh gateway request.
        b"gateway-brokered-request-denied" => "gateway-real-request-denied",
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected brokered mailbox gateway request body",
            ));
        }
    };
    let response = PrivateHttpResponseMessage {
        status: 201,
        headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
        body: b"gateway-brokered-header-ok".to_vec(),
        mailbox_publication: Some(PrivateMailboxPublicationMessage {
            slot: "deliver".to_owned(),
            method: "POST".to_owned(),
            route: "/gateway/golden-proof".to_owned(),
            headers: vec![("x-gateway-fixture".to_owned(), "real".to_owned())],
            content_type: Some("application/octet-stream".to_owned()),
            trace_context: None,
            body: b"gateway-real-mailbox-body".to_vec(),
            deduplication_key: deduplication_key.to_owned(),
        }),
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

/// Minimal long-lived service fixture for the private guest-loopback bridge.
///
/// The fixture uses a fixed worker pool and bounded queue, then stops after a
/// bounded request count. That keeps later transport tests deterministic while
/// still proving concurrent requests reach one persistent process.
fn serve_service() -> io::Result<()> {
    if std::env::var(SERVICE_ISOLATION_CHECK_ENV).as_deref() == Ok("1") {
        verify_private_service_isolation()?;
    }
    let port = service_port()?;
    let startup_delay = service_delay_from_env("HEPH_SERVICE_STARTUP_DELAY_MS")?;
    let request_delay = service_delay_from_env("HEPH_SERVICE_REQUEST_DELAY_MS")?;
    if !startup_delay.is_zero() {
        std::thread::sleep(startup_delay);
    }
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let startup_id = Arc::new(service_startup_id()?);
    println!(
        "service=ready pid={} startup_id={} port={port}",
        process::id(),
        startup_id
    );
    let request_count = Arc::new(AtomicUsize::new(0));
    let (sender, receiver) = mpsc::sync_channel(SERVICE_MAX_CONNECTIONS);
    let receiver = Arc::new(Mutex::new(receiver));
    for worker_number in 0..SERVICE_MAX_CONNECTIONS {
        let receiver = Arc::clone(&receiver);
        let startup_id = Arc::clone(&startup_id);
        thread::Builder::new()
            .name(format!("service-worker-{worker_number}"))
            .spawn(move || {
                loop {
                    let job = receiver
                        .lock()
                        .expect("service worker queue mutex is not poisoned")
                        .recv();
                    let Ok((stream, request_number)) = job else {
                        break;
                    };
                    if let Err(error) = serve_service_connection(
                        stream,
                        startup_id.as_str(),
                        request_number,
                        request_delay,
                    ) {
                        eprintln!("service connection failed: {error}");
                    }
                }
            })
            .map_err(io::Error::other)?;
    }
    loop {
        let (stream, _) = match listener.accept() {
            Ok(connection) => connection,
            Err(error) => {
                eprintln!("service accept failed: {error}");
                continue;
            }
        };
        let request_number = request_count.fetch_add(1, Ordering::Relaxed) + 1;
        if request_number > SERVICE_MAX_REQUESTS {
            return Err(io::Error::other("service fixture request limit reached"));
        }
        if sender.send((stream, request_number)).is_err() {
            return Err(io::Error::other("service worker queue closed"));
        }
    }
}

fn verify_private_service_isolation() -> io::Result<()> {
    if std::env::var_os(RUNTIME_AUTHORITY_ENV).is_some() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private service received a runtime authority environment path",
        ));
    }
    if Path::new(RUNTIME_AUTHORITY_FILE).exists() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private service found a runtime authority file",
        ));
    }
    expect_network_disabled().map_err(|error| io::Error::other(error.to_string()))?;
    println!("private-service-isolation=ok");
    Ok(())
}

fn serve_service_connection(
    mut stream: TcpStream,
    startup_id: &str,
    request_number: usize,
    request_delay: Duration,
) -> io::Result<()> {
    let target = match read_service_target(&mut stream) {
        Ok(target) => target,
        Err(error) => {
            if let Err(response_error) =
                write_service_response(&mut stream, 400, "text/plain", b"bad request")
            {
                eprintln!("service malformed-request response failed: {response_error}");
            }
            eprintln!("service request rejected: {error}");
            return Ok(());
        }
    };
    let delay = match service_delay_for_target(&target, request_delay) {
        Ok(delay) => delay,
        Err(error) => {
            if let Err(response_error) =
                write_service_response(&mut stream, 400, "text/plain", b"bad delay")
            {
                eprintln!("service bad-delay response failed: {response_error}");
            }
            eprintln!("service delay rejected: {error}");
            return Ok(());
        }
    };
    if !delay.is_zero() {
        eprintln!("private-service-delay=started target={target}");
        thread::sleep(delay);
    }
    let path = target
        .split_once('?')
        .map_or(target.as_str(), |(path, _)| path);
    match path {
        "/readyz" => write_service_response(&mut stream, 200, "text/plain", b"ready"),
        "/healthz" => write_service_response(&mut stream, 200, "text/plain", b"healthy"),
        // The daemon's public gateway namespace is intentionally preserved
        // across the private exchange. Accept the exact public service path
        // as well as the direct loopback path used by private-service tests.
        "/identity" | "/gateway/service/identity" => {
            let body = format!(
                r#"{{"pid":{},"startup_id":"{}","request_count":{request_number}}}"#,
                process::id(),
                startup_id
            );
            write_service_response(&mut stream, 200, "application/json", body.as_bytes())
        }
        "/crash" => {
            if let Err(error) = write_service_response(&mut stream, 503, "text/plain", b"crashing")
            {
                eprintln!("service crash response failed: {error}");
            }
            process::exit(SERVICE_CRASH_EXIT_CODE);
        }
        _ if path.starts_with("/delay/") => {
            write_service_response(&mut stream, 200, "text/plain", b"delayed")
        }
        _ => write_service_response(&mut stream, 404, "text/plain", b"not found"),
    }
}

fn service_port() -> io::Result<u16> {
    let value =
        std::env::var("HEPH_SERVICE_PORT").unwrap_or_else(|_| SERVICE_DEFAULT_PORT.to_string());
    let port = value.parse::<u16>().map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("HEPH_SERVICE_PORT is invalid: {error}"),
        )
    })?;
    if port < 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "HEPH_SERVICE_PORT must be between 1024 and 65535",
        ));
    }
    Ok(port)
}

fn service_delay_from_env(name: &str) -> io::Result<Duration> {
    let Some(value) = std::env::var_os(name) else {
        return Ok(Duration::ZERO);
    };
    let milliseconds = value
        .to_str()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{name} must be valid UTF-8 milliseconds"),
            )
        })?
        .parse::<u64>()
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{name} is invalid: {error}"),
            )
        })?;
    if milliseconds > SERVICE_MAX_DELAY_MS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} exceeds {SERVICE_MAX_DELAY_MS}ms"),
        ));
    }
    Ok(Duration::from_millis(milliseconds))
}

fn service_delay_for_target(target: &str, default: Duration) -> io::Result<Duration> {
    let path = target.split_once('?').map_or(target, |(path, _)| path);
    let Some(value) = path.strip_prefix("/delay/") else {
        return Ok(default);
    };
    let milliseconds = value.parse::<u64>().map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("service delay path is invalid: {error}"),
        )
    })?;
    if milliseconds > SERVICE_MAX_DELAY_MS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("service delay exceeds {SERVICE_MAX_DELAY_MS}ms"),
        ));
    }
    Ok(Duration::from_millis(milliseconds))
}

fn service_startup_id() -> io::Result<String> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    Ok(format!("{}-{timestamp}", process::id()))
}

fn read_service_target(stream: &mut TcpStream) -> io::Result<String> {
    stream.set_read_timeout(Some(SERVICE_IO_TIMEOUT))?;
    let mut request = Vec::with_capacity(1_024);
    let header_end = loop {
        let mut chunk = [0_u8; 1_024];
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "service request ended before headers",
            ));
        }
        request.extend_from_slice(&chunk[..read]);
        if request.len() > SERVICE_MAX_REQUEST_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "service request exceeds size limit",
            ));
        }
        if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    if request.len() > header_end {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "service fixture does not accept request bodies",
        ));
    }
    let headers = std::str::from_utf8(&request[..header_end])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "service request is not UTF-8"))?;
    let mut lines = headers.split("\r\n");
    let request_line = lines.next().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "service request line missing")
    })?;
    let mut parts = request_line.split_ascii_whitespace();
    let method = parts.next();
    let target = parts.next();
    let version = parts.next();
    if method != Some("GET")
        || target.is_none_or(|target| !target.starts_with('/'))
        || !matches!(version, Some("HTTP/1.0" | "HTTP/1.1"))
        || parts.next().is_some()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "service request line is unsupported",
        ));
    }
    let mut header_count = 0_usize;
    for line in lines {
        if line.is_empty() {
            break;
        }
        if line.len() > SERVICE_MAX_HEADER_LINE_BYTES || line.split_once(':').is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "service request header is invalid",
            ));
        }
        header_count += 1;
        if header_count > SERVICE_MAX_HEADER_COUNT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "service request has too many headers",
            ));
        }
        let (name, value) = line.split_once(':').expect("header was checked above");
        if name.eq_ignore_ascii_case("upgrade")
            || name.eq_ignore_ascii_case("transfer-encoding")
            || (name.eq_ignore_ascii_case("connection")
                && value
                    .split(',')
                    .any(|token| token.trim().eq_ignore_ascii_case("upgrade")))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "service fixture does not accept protocol upgrades",
            ));
        }
        if name.eq_ignore_ascii_case("content-length") {
            let length = value.trim().parse::<usize>().map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("service content length is invalid: {error}"),
                )
            })?;
            if length > SERVICE_MAX_BODY_BYTES || length != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "service fixture accepts only empty GET bodies",
                ));
            }
        }
    }
    Ok(target.expect("target was checked above").to_owned())
}

fn write_service_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        503 => "Service Unavailable",
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported fixture status",
            ));
        }
    };
    if body.len() > SERVICE_MAX_BODY_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "service response exceeds size limit",
        ));
    }
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()?;
    stream.shutdown(Shutdown::Write)
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
    let authority_path = std::env::var(vm_libkrun::protocol::RUNTIME_AUTHORITY_PATH_ENV)
        .unwrap_or_else(|_| String::from(vm_libkrun::protocol::GUEST_RUNTIME_AUTHORITY_PATH));
    let authority: GuestRuntimeAuthority = serde_json::from_slice(&fs::read(authority_path)?)?;
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
