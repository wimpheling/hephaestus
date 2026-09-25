use std::{
    fs::File,
    io::{self, Read},
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
};
use vm_libkrun::protocol::{
    GuestMessage, HostMessage, MAX_GATEWAY_HANDLER_OUTPUT_BYTES, MAX_PRIVATE_HTTP_BODY_BYTES,
    MAX_PRIVATE_HTTP_HEADERS, PrivateHttpRequestMessage, PrivateHttpResponseMessage,
};

use crate::{AGENT_GID, AGENT_UID, lock, read_frame, signal_process, write_message};

#[derive(Default)]
struct GatewayHandlerState {
    active: bool,
    child_pid: Option<u32>,
    cancelled: bool,
}

pub fn gateway_handler_loop(
    mut control: File,
    writer: &Arc<Mutex<File>>,
    command: &vm_libkrun::protocol::GuestCommandMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = Arc::new(Mutex::new(GatewayHandlerState::default()));
    while let Ok(message) = read_frame::<HostMessage>(&mut control) {
        match message {
            HostMessage::PrivateHttpRequest {
                request_id,
                request,
            } => {
                validate_private_http_request(&request)?;
                let mut current = lock(&state);
                if current.active {
                    return Err("received concurrent private HTTP requests".into());
                }
                current.active = true;
                current.cancelled = false;
                drop(current);
                let writer = Arc::clone(writer);
                let state = Arc::clone(&state);
                let command = command.clone();
                thread::spawn(move || {
                    let result = invoke_gateway_handler(&command, &request, &state);
                    let message = match result {
                        Ok(response) => GuestMessage::PrivateHttpResponse {
                            request_id,
                            response,
                        },
                        Err(error) => GuestMessage::Error {
                            code: String::from("gateway-handler"),
                            message: bounded_error_message(&error),
                        },
                    };
                    let _write_result = write_message(&writer, &message);
                });
            }
            HostMessage::Cancel { .. } => {
                let pid = {
                    let mut current = lock(&state);
                    current.cancelled = true;
                    current.child_pid
                };
                if let Some(pid) = pid {
                    let _signal_result = signal_process(pid, libc::SIGTERM);
                }
            }
            HostMessage::HealthPing { nonce } => {
                write_message(writer, &GuestMessage::Health { nonce })?;
            }
            HostMessage::Start { .. } => return Err("received duplicate start command".into()),
            _ => return Err("received unsupported gateway control message".into()),
        }
    }
    Ok(())
}

fn invoke_gateway_handler(
    command: &vm_libkrun::protocol::GuestCommandMessage,
    request: &PrivateHttpRequestMessage,
    state: &Arc<Mutex<GatewayHandlerState>>,
) -> io::Result<PrivateHttpResponseMessage> {
    let mut child = Command::new(&command.program);
    child
        .args(&command.args)
        .env_clear()
        .envs(&command.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .uid(AGENT_UID)
        .gid(AGENT_GID);
    if let Some(working_dir) = &command.working_dir {
        child.current_dir(working_dir);
    }
    let mut child = child.spawn()?;
    let pid = child.id();
    {
        let mut current = lock(state);
        current.child_pid = Some(pid);
        if current.cancelled {
            let _signal_result = signal_process(pid, libc::SIGTERM);
        }
    }
    let outcome = (|| {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("handler stdin is unavailable"))?;
        ciborium::into_writer(request, &mut stdin).map_err(io::Error::other)?;
        drop(stdin);
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("handler stdout is unavailable"))?;
        let mut output = Vec::with_capacity(MAX_PRIVATE_HTTP_BODY_BYTES.min(8_192));
        stdout
            .take(u64::try_from(MAX_GATEWAY_HANDLER_OUTPUT_BYTES + 1).unwrap())
            .read_to_end(&mut output)?;
        if output.len() > MAX_GATEWAY_HANDLER_OUTPUT_BYTES {
            // Stop a malicious handler before waiting: leaving bytes in its
            // stdout pipe could otherwise deadlock the one-request VM.
            let _signal_result = signal_process(pid, libc::SIGKILL);
            let _status = child.wait()?;
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "gateway handler response exceeds protocol limit",
            ));
        }
        let status = child.wait()?;
        if !status.success() {
            return Err(io::Error::other("gateway handler exited unsuccessfully"));
        }
        let response: PrivateHttpResponseMessage =
            ciborium::from_reader(output.as_slice()).map_err(io::Error::other)?;
        validate_private_http_response(&response)?;
        Ok(response)
    })();
    let mut current = lock(state);
    current.active = false;
    current.child_pid = None;
    drop(current);
    if outcome.is_err() {
        // Every early stdin/stdout/CBOR failure must reap the direct child;
        // otherwise a broken handler pipe leaves a process behind until the
        // microVM is forcibly destroyed.
        let _signal_result = signal_process(pid, libc::SIGKILL);
        let _wait_result = child.wait();
    }
    outcome
}

fn validate_private_http_request(request: &PrivateHttpRequestMessage) -> io::Result<()> {
    if request.method.is_empty()
        || request
            .method
            .bytes()
            .any(|byte| !byte.is_ascii_uppercase())
        || !request.path_and_query.starts_with('/')
        || request.path_and_query.contains('\0')
        || request.headers.len() > MAX_PRIVATE_HTTP_HEADERS
        || request.body.len() > MAX_PRIVATE_HTTP_BODY_BYTES
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "private HTTP request violates gateway contract",
        ));
    }
    Ok(())
}

fn validate_private_http_response(response: &PrivateHttpResponseMessage) -> io::Result<()> {
    if !(100..=599).contains(&response.status)
        || response.headers.len() > MAX_PRIVATE_HTTP_HEADERS
        || response.body.len() > MAX_PRIVATE_HTTP_BODY_BYTES
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "private HTTP response violates gateway contract",
        ));
    }
    if let Some(publication) = &response.mailbox_publication {
        validate_mailbox_publication(publication)?;
    }
    Ok(())
}

fn validate_mailbox_publication(
    publication: &vm_libkrun::protocol::PrivateMailboxPublicationMessage,
) -> io::Result<()> {
    use vm_libkrun::protocol::{
        MAX_MAILBOX_PUBLICATION_BODY_BYTES, MAX_MAILBOX_PUBLICATION_CONTENT_TYPE_BYTES,
        MAX_MAILBOX_PUBLICATION_DEDUPLICATION_KEY_BYTES, MAX_MAILBOX_PUBLICATION_HEADER_NAME_BYTES,
        MAX_MAILBOX_PUBLICATION_HEADER_VALUE_BYTES, MAX_MAILBOX_PUBLICATION_HEADERS,
        MAX_MAILBOX_PUBLICATION_METHOD_BYTES, MAX_MAILBOX_PUBLICATION_ROUTE_BYTES,
        MAX_MAILBOX_PUBLICATION_SLOT_BYTES, MAX_MAILBOX_PUBLICATION_TRACE_CONTEXT_BYTES,
    };
    let bounded =
        |value: &str, maximum| !value.is_empty() && value.len() <= maximum && !value.contains('\0');
    let valid_method = !publication.method.is_empty()
        && publication.method.len() <= MAX_MAILBOX_PUBLICATION_METHOD_BYTES
        && publication
            .method
            .bytes()
            .all(|byte| byte.is_ascii_uppercase());
    let valid_optional = |value: &Option<String>, maximum| {
        value
            .as_ref()
            .is_none_or(|value| value.len() <= maximum && !value.contains('\0'))
    };
    if !bounded(&publication.slot, MAX_MAILBOX_PUBLICATION_SLOT_BYTES)
        || !valid_method
        || !bounded(&publication.route, MAX_MAILBOX_PUBLICATION_ROUTE_BYTES)
        || !publication.route.starts_with('/')
        || publication.headers.len() > MAX_MAILBOX_PUBLICATION_HEADERS
        || publication.headers.iter().any(|(name, value)| {
            !bounded(name, MAX_MAILBOX_PUBLICATION_HEADER_NAME_BYTES)
                || value.len() > MAX_MAILBOX_PUBLICATION_HEADER_VALUE_BYTES
                || value.contains('\0')
        })
        || !valid_optional(
            &publication.content_type,
            MAX_MAILBOX_PUBLICATION_CONTENT_TYPE_BYTES,
        )
        || !valid_optional(
            &publication.trace_context,
            MAX_MAILBOX_PUBLICATION_TRACE_CONTEXT_BYTES,
        )
        || publication.body.len() > MAX_MAILBOX_PUBLICATION_BODY_BYTES
        || !bounded(
            &publication.deduplication_key,
            MAX_MAILBOX_PUBLICATION_DEDUPLICATION_KEY_BYTES,
        )
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "mailbox publication violates gateway contract",
        ));
    }
    Ok(())
}

fn bounded_error_message(error: &io::Error) -> String {
    let mut message = error.to_string();
    message.truncate(1_024);
    message
}
