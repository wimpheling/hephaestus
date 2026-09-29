use std::{
    io::{self, ErrorKind, Write},
    net::TcpStream,
};

use super::{
    isolation::isolation_response,
    types::{
        EXPECTED_GATEWAY_AUTHORITY, MAX_HEADER_COUNT, MAX_HEADER_LINE_BYTES, RequestMetadata,
        Response, StartupIdentity,
    },
};

pub(crate) fn response_for(request: &[u8], identity: &StartupIdentity) -> Response {
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

pub(crate) fn write_response(stream: &mut TcpStream, response: &Response) -> io::Result<()> {
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

pub(crate) fn bad_request() -> Response {
    Response {
        status: 400,
        content_type: "text/plain; charset=utf-8",
        body: b"bad request".to_vec(),
    }
}
