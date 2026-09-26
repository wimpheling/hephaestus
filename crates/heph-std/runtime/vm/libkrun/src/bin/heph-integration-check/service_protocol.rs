use std::{
    io::{self, Read, Write},
    net::{Shutdown, TcpStream},
    process,
    time::Duration,
};

use crate::{
    SERVICE_DEFAULT_PORT, SERVICE_IO_TIMEOUT, SERVICE_MAX_BODY_BYTES, SERVICE_MAX_DELAY_MS,
    SERVICE_MAX_HEADER_COUNT, SERVICE_MAX_HEADER_LINE_BYTES, SERVICE_MAX_REQUEST_BYTES,
};

// HTTP parsing and response helpers for the bounded private-service fixture.
pub fn emit_service_log_markers() -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(b"service-log-stdout=ordinary\n")?;
    stdout.flush()?;
    drop(stdout);

    let mut stderr = io::stderr().lock();
    stderr.write_all(b"service-log-stderr=ordinary\n")?;
    stderr.flush()
}

pub fn service_port() -> io::Result<u16> {
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

pub fn service_delay_from_env(name: &str) -> io::Result<Duration> {
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

pub fn service_delay_for_target(target: &str, default: Duration) -> io::Result<Duration> {
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

pub fn service_startup_id() -> io::Result<String> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    Ok(format!("{}-{timestamp}", process::id()))
}

pub fn read_service_target(stream: &mut TcpStream) -> io::Result<String> {
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

pub fn write_service_response(
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
