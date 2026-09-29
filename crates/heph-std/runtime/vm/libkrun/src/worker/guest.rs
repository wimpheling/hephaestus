use super::{
    lock, send_message,
    types::{WireError, WireErrorKind, WorkerEvent, WorkerMessage},
};
use crate::{
    framing::{read_sync, write_sync},
    protocol::{
        GATEWAY_HANDLER_CONTRACT_LABEL, GATEWAY_HANDLER_CONTRACT_V1, GuestCommandMessage,
        GuestMessage, GuestMount, GuestStateVolume, HostMessage, MAX_LOG_CHUNK_SIZE,
        MAX_METRIC_LABELS, MAX_METRIC_TEXT_SIZE, MAX_PRIVATE_HTTP_BODY_BYTES,
        MAX_PRIVATE_HTTP_HEADERS, MAX_RESULT_MESSAGE_SIZE, PROTOCOL_VERSION,
        PrivateHttpServiceMessage, RUNTIME_AUTHORITY_PATH_ENV, RuntimeAuthorityMessage,
        RuntimeGitBridgeMessage,
    },
    validation::PreparedSpec,
};
use std::{
    collections::BTreeMap,
    error::Error,
    io,
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
};

pub(super) fn spawn_guest_control(
    listener: UnixListener,
    guest: Arc<Mutex<Option<UnixStream>>>,
    spec: PreparedSpec,
    writer: Arc<Mutex<UnixStream>>,
) {
    thread::spawn(move || {
        let result = handle_guest(&listener, &guest, spec, &writer);
        if let Err(error) = result {
            tracing::error!(%error, "guest control channel failed");
            let failure = WireError {
                kind: WireErrorKind::Backend,
                code: "guest-control".to_owned(),
                message: error.to_string(),
            };
            let _failure_result = send_message(
                &writer,
                &WorkerMessage::Event(WorkerEvent::BackendFailure(failure)),
            );
            let _exit_result = send_message(
                &writer,
                &WorkerMessage::Event(WorkerEvent::Exited {
                    code: None,
                    signal: None,
                }),
            );
        }
    });
}

// Keeping the authenticated handshake, exact authority acknowledgement, and
// event forwarding together makes their ordering directly auditable.
#[allow(clippy::too_many_lines)]
fn handle_guest(
    listener: &UnixListener,
    guest_slot: &Arc<Mutex<Option<UnixStream>>>,
    spec: PreparedSpec,
    writer: &Arc<Mutex<UnixStream>>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (mut stream, _) = listener.accept()?;
    let hello: GuestMessage = read_sync(&mut stream)?;
    if !matches!(
        hello,
        GuestMessage::Hello {
            version: PROTOCOL_VERSION
        }
    ) {
        return Err(Box::new(io::Error::new(
            io::ErrorKind::InvalidData,
            "guest protocol version mismatch",
        )));
    }

    let guest_writer = stream.try_clone()?;
    *lock(guest_slot) = Some(guest_writer);
    let mut command = GuestCommandMessage {
        program: spec.command.program,
        args: spec.command.args,
        env: spec.command.env,
        working_dir: spec.command.working_dir,
    };
    let mounts = spec
        .mounts
        .into_iter()
        .map(|mount| GuestMount {
            tag: mount.tag,
            guest_path: mount.guest_path,
            read_only: mount.read_only,
        })
        .collect();
    let state_volume = match volume_labels(&spec.labels) {
        Some((filesystem_uuid, guest_path)) => Some(GuestStateVolume {
            filesystem_uuid: filesystem_uuid.clone(),
            guest_path: PathBuf::from(guest_path),
        }),
        _ => None,
    };
    let runtime_authority = spec.runtime_authority.map(|authority| {
        Box::new(RuntimeAuthorityMessage {
            session_id: authority.session_id,
            generation: authority.generation,
            credential: authority.credential,
            runtime_git_credential: authority.runtime_git_credential,
        })
    });
    if let Some(authority) = runtime_authority.as_ref() {
        command.env.insert(
            String::from(RUNTIME_AUTHORITY_PATH_ENV),
            format!(
                "/run/hephaestus-authority/session-{}.json",
                authority.session_id
            ),
        );
    }
    let gateway_handler = spec
        .labels
        .get(GATEWAY_HANDLER_CONTRACT_LABEL)
        .is_some_and(|value| value == GATEWAY_HANDLER_CONTRACT_V1);
    let private_http_service =
        spec.private_http_service
            .as_ref()
            .map(|service| PrivateHttpServiceMessage {
                loopback_port: service.loopback_port,
                max_connections: service.max_connections,
                connect_timeout_ms: service.connect_timeout_ms,
            });
    let runtime_git_bridge =
        spec.runtime_git_bridge
            .as_ref()
            .map(|bridge| RuntimeGitBridgeMessage {
                repository_id: bridge.repository_id,
                loopback_port: bridge.loopback_port,
            });
    let expected_authority_ack = runtime_authority
        .as_ref()
        .map(|authority| (authority.session_id, authority.generation));
    write_sync(
        &mut stream,
        &HostMessage::Start {
            version: PROTOCOL_VERSION,
            command,
            mounts,
            state_volume,
            runtime_authority,
            gateway_handler,
            private_http_service,
            runtime_git_bridge,
        },
    )?;

    let mut authority_acknowledged = false;
    loop {
        let message: GuestMessage = read_sync(&mut stream)?;
        validate_guest_message(&message)?;
        validate_authority_sequence(
            expected_authority_ack,
            &mut authority_acknowledged,
            &message,
        )?;
        let event = match message {
            GuestMessage::Hello { .. } => continue,
            GuestMessage::Ready => WorkerEvent::Ready,
            GuestMessage::RuntimeAuthorityAcknowledged {
                session_id,
                generation,
            } => WorkerEvent::RuntimeAuthorityAcknowledged {
                session_id,
                generation,
            },
            GuestMessage::Log { stream, bytes } => WorkerEvent::Log {
                stream: stream.into(),
                bytes,
            },
            GuestMessage::Metric {
                name,
                value,
                labels,
            } => WorkerEvent::Metric {
                name,
                value,
                labels,
            },
            GuestMessage::Health { nonce } => WorkerEvent::Health { nonce },
            GuestMessage::FinalizeResult { message } => WorkerEvent::FinalizeResult { message },
            GuestMessage::Exited { code, signal } => WorkerEvent::Exited { code, signal },
            GuestMessage::Error { code, message } => WorkerEvent::BackendFailure(WireError {
                kind: WireErrorKind::Backend,
                code,
                message,
            }),
            GuestMessage::PrivateHttpResponse {
                request_id,
                response,
            } => WorkerEvent::PrivateHttpResponse {
                request_id,
                response,
            },
        };
        let exited = matches!(event, WorkerEvent::Exited { .. });
        send_message(writer, &WorkerMessage::Event(event))?;
        if exited {
            break;
        }
    }
    Ok(())
}

fn volume_labels(labels: &BTreeMap<String, String>) -> Option<(&String, &String)> {
    let agent = (
        labels.get("hephaestus.agent-state.filesystem-uuid"),
        labels.get("hephaestus.agent-state.mount-path"),
    );
    let scratch = (
        labels.get("hephaestus.oci-scratch.filesystem-uuid"),
        labels.get("hephaestus.oci-scratch.mount-path"),
    );
    match (agent, scratch) {
        ((Some(uuid), Some(path)), (None, None)) | ((None, None), (Some(uuid), Some(path))) => {
            Some((uuid, path))
        }
        _ => None,
    }
}

pub(super) fn validate_authority_sequence(
    expected: Option<(uuid::Uuid, u64)>,
    acknowledged: &mut bool,
    message: &GuestMessage,
) -> io::Result<()> {
    match message {
        GuestMessage::RuntimeAuthorityAcknowledged {
            session_id,
            generation,
        } => {
            if *acknowledged || expected != Some((*session_id, *generation)) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "guest runtime authority acknowledgement does not match bootstrap",
                ));
            }
            *acknowledged = true;
        }
        GuestMessage::Ready if expected.is_some() && !*acknowledged => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "guest became ready before runtime authority acknowledgement",
            ));
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn validate_guest_message(message: &GuestMessage) -> io::Result<()> {
    match message {
        GuestMessage::Hello { .. } => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "guest sent a duplicate readiness handshake",
        )),
        GuestMessage::Log { bytes, .. } if bytes.len() > MAX_LOG_CHUNK_SIZE => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "guest log chunk exceeds protocol limit",
        )),
        GuestMessage::RuntimeAuthorityAcknowledged { generation: 0, .. } => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "guest runtime authority acknowledgement generation must be positive",
        )),
        GuestMessage::Metric { name, .. }
            if name.is_empty() || name.len() > MAX_METRIC_TEXT_SIZE =>
        {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "guest metric name is empty or exceeds protocol limit",
            ))
        }
        GuestMessage::Metric { labels, .. } if labels.len() > MAX_METRIC_LABELS => {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "guest metric label count exceeds protocol limit",
            ))
        }
        GuestMessage::Metric { labels, .. }
            if labels.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > MAX_METRIC_TEXT_SIZE
                    || value.len() > MAX_METRIC_TEXT_SIZE
            }) =>
        {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "guest metric label is empty or exceeds protocol limit",
            ))
        }
        GuestMessage::FinalizeResult { message }
            if message.len() > MAX_RESULT_MESSAGE_SIZE || message.contains('\0') =>
        {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "guest result message exceeds protocol limit or contains NUL",
            ))
        }
        GuestMessage::Exited {
            code: Some(_),
            signal: Some(_),
        } => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "guest exit cannot contain both code and signal",
        )),
        GuestMessage::PrivateHttpResponse { response, .. }
            if !(100..=599).contains(&response.status)
                || response.body.len() > MAX_PRIVATE_HTTP_BODY_BYTES
                || response.headers.len() > MAX_PRIVATE_HTTP_HEADERS =>
        {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "private HTTP response exceeds protocol limits",
            ))
        }
        _ => Ok(()),
    }
}
