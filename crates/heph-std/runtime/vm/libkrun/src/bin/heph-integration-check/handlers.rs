use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use std::{
    fs,
    io::{self, Read, Write},
    net::TcpStream,
    process::{Command, Stdio},
};
use vm_libkrun::protocol::{
    PrivateHttpRequestMessage, PrivateHttpResponseMessage, PrivateMailboxPublicationMessage,
};

use crate::RUNTIME_GIT_CREDENTIAL_HELPER;

/// Exercises the guest-local helper and loopback proxy with no guest IP
/// networking. The host peer is a test-only HTTP endpoint; this does not
/// assert production Git authentication or repository protocol behavior.
pub fn runtime_git_http(repository: &str) -> io::Result<()> {
    let mut helper = Command::new(RUNTIME_GIT_CREDENTIAL_HELPER)
        .arg("get")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    helper
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("runtime Git helper stdin unavailable"))?
        .write_all(
            format!("protocol=http\nhost=127.0.0.1:19100\npath=/{repository}\n\n").as_bytes(),
        )?;
    let credentials = helper.wait_with_output()?;
    if !credentials.status.success() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "runtime Git helper rejected the bound route",
        ));
    }
    let credentials = String::from_utf8(credentials.stdout)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "runtime Git credentials UTF-8"))?;
    let username = credentials
        .lines()
        .find_map(|line| line.strip_prefix("username="))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "runtime Git username"))?;
    let password = credentials
        .lines()
        .find_map(|line| line.strip_prefix("password="))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "runtime Git password"))?;
    let authorization = BASE64.encode(format!("{username}:{password}"));
    let mut stream = TcpStream::connect(("127.0.0.1", 19_100))?;
    stream.write_all(
        format!(
            "GET /{repository}/runtime-git-proof HTTP/1.1\r\nHost: 127.0.0.1:19100\r\nAuthorization: Basic {authorization}\r\nConnection: close\r\n\r\n"
        )
        .as_bytes(),
    )?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    if !response.starts_with(b"HTTP/1.1 200 OK\r\n") || !response.ends_with(b"runtime-git-response")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "runtime Git bridge response was not exact (bytes={}, starts={}, ends={})",
                response.len(),
                response.starts_with(b"HTTP/1.1 200 OK\r\n"),
                response.ends_with(b"runtime-git-response")
            ),
        ));
    }
    println!("runtime-git-http=ok");
    Ok(())
}

pub fn expect_mailbox() -> Result<(), Box<dyn std::error::Error>> {
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
pub fn private_http_handler() -> io::Result<()> {
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
pub fn private_http_brokered_header_handler() -> io::Result<()> {
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
pub fn private_http_brokered_mailbox_handler() -> io::Result<()> {
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
