use std::{
    io::{self, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
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

use crate::{
    RUNTIME_AUTHORITY_ENV, RUNTIME_AUTHORITY_FILE, SERVICE_CRASH_EXIT_CODE, SERVICE_HOLD,
    SERVICE_ISOLATION_CHECK_ENV, SERVICE_MAX_CONNECTIONS, SERVICE_MAX_REQUESTS,
    broker::expect_network_disabled,
    service_protocol::{
        emit_service_log_markers, read_service_target, service_delay_for_target,
        service_delay_from_env, service_port, service_startup_id, write_service_response,
    },
};

pub fn serve_http() -> io::Result<()> {
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
pub fn serve_service() -> io::Result<()> {
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
        // This bounded fixture-only response keeps one accepted gateway
        // invocation in flight while a replacement revision becomes ready.
        // The public exchange deadline remains the authority for the bound.
        "/gateway/service/hold" => {
            thread::sleep(SERVICE_HOLD);
            let body = format!(
                r#"{{"pid":{},"startup_id":"{}","request_count":{request_number}}}"#,
                process::id(),
                startup_id
            );
            write_service_response(&mut stream, 200, "application/json", body.as_bytes())
        }
        "/gateway/service/log" => {
            emit_service_log_markers()?;
            write_service_response(&mut stream, 200, "text/plain", b"log-emitted")
        }
        "/crash" | "/gateway/service/crash" => {
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
