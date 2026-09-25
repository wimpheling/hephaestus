use super::super::{
    GitHttpService, Principal,
    auth::capability_operation,
    backend::{
        BackendEnvironment, backend_command, header_value, header_value_name, hidden_runtime_refs,
        materialize_receive_context, read_cgi_headers, send_stream_error, stream_response,
    },
    errors::{error_response, service_error},
    receive_policy,
    refs::{diff_refs, snapshot_refs},
};
use axum::{
    body::Body,
    http::{Response, StatusCode},
};
use bytes::Bytes;
use forge_domain::{ReceiveId, Repository, RuntimeReceiveProvenance};
use futures_util::StreamExt;
use identity_domain::AuthenticatedIdentity;
use std::{io, path::Path, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};
use tokio_stream::wrappers::ReceiverStream;

pub(super) struct BackendExecutionInput {
    pub(super) service: Arc<GitHttpService>,
    pub(super) repository_id: forge_domain::RepositoryId,
    pub(super) operation: super::super::GitOperation,
    pub(super) endpoint: &'static str,
    pub(super) query: Option<String>,
    pub(super) principal: Principal,
    pub(super) request: axum::http::Request<Body>,
    pub(super) receive: bool,
    pub(super) principal_identity: Option<AuthenticatedIdentity>,
    pub(super) runtime_receive_provenance: Option<RuntimeReceiveProvenance>,
    pub(super) runtime_receive_context: Option<receive_policy::ResolvedRuntimeReceiveContext>,
    pub(super) repository: Repository,
}

// Keep backend startup, bounded streaming, and receive durability in one transaction sequence.
#[allow(clippy::too_many_lines)]
pub(super) async fn execute_backend(input: BackendExecutionInput) -> Response<Body> {
    let BackendExecutionInput {
        service,
        repository_id,
        operation,
        endpoint,
        query,
        principal,
        request,
        receive,
        principal_identity,
        runtime_receive_provenance,
        runtime_receive_context,
        repository,
    } = input;
    let runtime_scope = match &principal {
        Principal::Human(_) => None,
        Principal::Runtime(runtime) => runtime
            .git_authority()
            .map(|authority| Arc::clone(&authority.scope)),
    };
    // Runtime ref visibility is computed from canonical refs. Serialize it
    // with receives so a concurrent human push cannot introduce an
    // out-of-scope advertisement after filtering but before upload-pack.
    let receive_guard = if receive || runtime_scope.is_some() {
        Some(service.lock_receive(repository_id).await)
    } else {
        None
    };
    let hidden_refs = if let Some(scope) = runtime_scope.as_deref() {
        match hidden_runtime_refs(
            service.storage.repository_path(repository_id),
            scope,
            capability_operation(operation),
        )
        .await
        {
            Ok(refs) => refs,
            Err(error) => return service_error(error),
        }
    } else {
        Vec::new()
    };
    let declared_content_length = request
        .headers()
        .get(http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let max_request_bytes =
        runtime_receive_context
            .as_ref()
            .map_or(service.limits.max_request_bytes, |context| {
                service
                    .limits
                    .max_request_bytes
                    .min(context.transfer_limits().request_bytes())
            });
    if let Some(content_length) = declared_content_length {
        if content_length > max_request_bytes {
            return error_response(StatusCode::PAYLOAD_TOO_LARGE, "Git request exceeds limit");
        }
    }

    let receive_id = receive.then(ReceiveId::new);
    let before = if receive {
        match snapshot_refs(service.storage.repository_path(repository_id)).await {
            Ok(refs) => Some(refs),
            Err(error) => return service_error(error),
        }
    } else {
        None
    };
    let hook_context = match runtime_receive_context
        .as_ref()
        .map(materialize_receive_context)
        .transpose()
    {
        Ok(context) => context,
        Err(error) => return service_error(error),
    };
    let runtime_hook_directory = runtime_receive_context.as_ref().and_then(|_| {
        service
            .runtime_receive_hook
            .as_deref()
            .and_then(Path::parent)
    });
    let request_bytes_bound = runtime_receive_context.as_ref().map(|_| {
        declared_content_length
            .unwrap_or(max_request_bytes)
            .to_string()
    });
    let repository_id_text = repository_id.to_string();
    let environment = BackendEnvironment {
        project_root: service.storage.root(),
        repository_id,
        endpoint,
        method: request.method().as_str(),
        query: query.as_deref().unwrap_or(""),
        remote_user: principal.name(),
        content_type: header_value(&request, http::header::CONTENT_TYPE),
        content_length: header_value(&request, http::header::CONTENT_LENGTH),
        git_protocol: header_value_name(&request, "git-protocol"),
        runtime_receive_hook_directory: runtime_hook_directory,
        runtime_receive_context_file: hook_context.as_ref().map(tempfile::NamedTempFile::path),
        runtime_receive_repository: runtime_receive_context
            .as_ref()
            .map(|_| repository_id_text.as_str()),
        runtime_receive_request_bytes: request_bytes_bound.as_deref(),
        hidden_refs: &hidden_refs,
    };
    let mut command = backend_command(&service.backend, &environment);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return service_error(error),
    };
    let Some(mut stdin) = child.stdin.take() else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Git backend has no stdin",
        );
    };
    let Some(mut stdout) = child.stdout.take() else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Git backend has no stdout",
        );
    };
    let Some(stderr) = child.stderr.take() else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Git backend has no stderr",
        );
    };

    let mut body_stream = request.into_body().into_data_stream();
    let request_writer = tokio::spawn(async move {
        let mut written = 0_u64;
        while let Some(chunk) = body_stream.next().await {
            let chunk = chunk.map_err(io::Error::other)?;
            written = written
                .checked_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX))
                .ok_or_else(|| io::Error::other("Git request length overflow"))?;
            if written > max_request_bytes {
                return Err(io::Error::other("Git request exceeds configured limit"));
            }
            stdin.write_all(&chunk).await?;
        }
        stdin.shutdown().await
    });
    let stderr_reader = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stderr.take(64 * 1024).read_to_end(&mut bytes).await?;
        Ok::<_, io::Error>(bytes)
    });

    let (status, response_headers, remainder) = match read_cgi_headers(&mut stdout).await {
        Ok(headers) => headers,
        Err(error) => return service_error(error),
    };
    let (sender, receiver) = mpsc::channel::<Result<Bytes, io::Error>>(8);
    // Keep the HTTP body open until backend completion and receive durability
    // are known, so a push cannot report transport completion first.
    let completion_sender = sender.clone();
    let max_response_bytes = service.limits.max_response_bytes;
    let response_reader = tokio::spawn(stream_response(
        stdout,
        sender,
        remainder,
        max_response_bytes,
    ));

    let repository_service = Arc::clone(&service.repository);
    let repository_for_receive = repository.clone();
    let repository_path = service.storage.repository_path(repository_id);
    let principal_name = principal.name().to_owned();
    let timeout = service.limits.transaction_timeout;
    tokio::spawn(async move {
        // Keep the owner-only authority file alive only while this exact
        // backend transaction and its hook descendants can use the handle.
        let runtime_receive_context_file = hook_context;
        let receive_guard = receive_guard;
        let completion = tokio::time::timeout(timeout, child.wait()).await;
        let status = match completion {
            Ok(Ok(status)) => status,
            Ok(Err(error)) => {
                send_stream_error(&completion_sender, error.to_string()).await;
                tracing::warn!(%repository_id, ?receive_id, %error, "Git backend wait failed");
                return;
            }
            Err(_) => {
                if let Err(error) = child.kill().await {
                    tracing::warn!(%repository_id, ?receive_id, %error, "timed-out Git backend could not be killed");
                }
                send_stream_error(&completion_sender, "Git transaction timed out").await;
                tracing::warn!(%repository_id, ?receive_id, "Git backend transaction timed out");
                return;
            }
        };
        let writer_result = request_writer.await;
        let reader_result = response_reader.await;
        let stderr = stderr_reader
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
        if !status.success()
            || !matches!(writer_result, Ok(Ok(())))
            || !matches!(reader_result, Ok(Ok(())))
        {
            send_stream_error(&completion_sender, "Git backend transaction failed").await;
            tracing::warn!(
                %repository_id,
                ?receive_id,
                backend_status = ?status.code(),
                stderr = %String::from_utf8_lossy(&stderr),
                "Git backend transaction failed"
            );
            return;
        }
        if let (Some(receive_id), Some(before)) = (receive_id, before) {
            match snapshot_refs(repository_path).await {
                Ok(after) => {
                    let updates = diff_refs(&before, &after);
                    if updates.is_empty() {
                        tracing::debug!(%repository_id, %receive_id, "push changed no refs");
                        return;
                    }
                    let result = if let Some(provenance) = runtime_receive_provenance {
                        repository_service
                            .accept_runtime_receive(
                                &repository_for_receive,
                                receive_id,
                                provenance,
                                &updates,
                            )
                            .await
                    } else {
                        repository_service
                            .accept_receive_as(
                                &repository_for_receive,
                                receive_id,
                                &principal_name,
                                principal_identity.as_ref(),
                                &updates,
                            )
                            .await
                    };
                    if let Err(error) = result {
                        send_stream_error(
                            &completion_sender,
                            format!("accepted receive persistence failed: {error}"),
                        )
                        .await;
                        tracing::error!(
                            %repository_id,
                            %receive_id,
                            %error,
                            "accepted Git receive could not be persisted"
                        );
                    } else {
                        tracing::info!(
                            %repository_id,
                            %receive_id,
                            ref_updates = updates.len(),
                            "accepted Git receive was persisted"
                        );
                    }
                }
                Err(error) => {
                    send_stream_error(
                        &completion_sender,
                        format!("accepted receive inspection failed: {error}"),
                    )
                    .await;
                    tracing::error!(
                        %repository_id,
                        %receive_id,
                        %error,
                        "accepted Git receive refs could not be inspected"
                    );
                }
            }
        }
        drop(receive_guard);
        drop(runtime_receive_context_file);
    });

    let mut builder = Response::builder().status(status);
    for (name, value) in &response_headers {
        builder = builder.header(name, value);
    }
    builder
        .body(Body::from_stream(ReceiverStream::new(receiver)))
        .unwrap_or_else(service_error)
}
