const MAX_CGI_HEADERS: usize = 32 * 1024;
use crate::{GitHttpError, errors::domain, receive_policy, refs::snapshot_refs};
use axum::{
    body::Body,
    http::{HeaderMap, HeaderName, HeaderValue, Request, StatusCode},
};
use bytes::{Bytes, BytesMut};
use forge_domain::RepositoryId;
use std::{
    io,
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::{
    io::AsyncReadExt,
    process::{ChildStdout, Command},
    sync::mpsc,
};

pub async fn send_stream_error(
    sender: &mpsc::Sender<Result<Bytes, io::Error>>,
    message: impl Into<String>,
) {
    let _ = sender.send(Err(io::Error::other(message.into()))).await;
}

pub fn header_value(request: &Request<Body>, header: HeaderName) -> Option<&str> {
    request
        .headers()
        .get(header)
        .and_then(|value| value.to_str().ok())
}

pub fn header_value_name<'request>(
    request: &'request Request<Body>,
    header: &str,
) -> Option<&'request str> {
    request
        .headers()
        .get(header)
        .and_then(|value| value.to_str().ok())
}

pub struct BackendEnvironment<'a> {
    pub project_root: &'a Path,
    pub repository_id: RepositoryId,
    pub endpoint: &'a str,
    pub method: &'a str,
    pub query: &'a str,
    pub remote_user: &'a str,
    pub content_type: Option<&'a str>,
    pub content_length: Option<&'a str>,
    pub git_protocol: Option<&'a str>,
    pub runtime_receive_hook_directory: Option<&'a Path>,
    pub runtime_receive_context_file: Option<&'a Path>,
    pub runtime_receive_repository: Option<&'a str>,
    pub runtime_receive_request_bytes: Option<&'a str>,
    pub hidden_refs: &'a [String],
}

pub fn backend_command(backend: &Path, environment: &BackendEnvironment<'_>) -> Command {
    let mut command = Command::new(backend);
    command
        .env_clear()
        .env("GIT_PROJECT_ROOT", environment.project_root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env(
            "PATH_INFO",
            format!(
                "/{}.git/{}",
                environment.repository_id, environment.endpoint
            ),
        )
        .env("REQUEST_METHOD", environment.method)
        .env("QUERY_STRING", environment.query)
        .env("GATEWAY_INTERFACE", "CGI/1.1")
        .env("SERVER_PROTOCOL", "HTTP/1.1")
        .env("REMOTE_USER", environment.remote_user)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (name, value) in [
        ("CONTENT_TYPE", environment.content_type),
        ("CONTENT_LENGTH", environment.content_length),
        ("GIT_PROTOCOL", environment.git_protocol),
    ] {
        if let Some(value) = value {
            command.env(name, value);
        }
    }
    let mut configuration = Vec::<(&str, String)>::new();
    if let Some(hook_directory) = environment.runtime_receive_hook_directory {
        configuration.push((
            "core.hooksPath",
            hook_directory.to_string_lossy().into_owned(),
        ));
    }
    for reference in environment.hidden_refs {
        configuration.push(("uploadpack.hideRefs", reference.clone()));
        configuration.push(("receive.hideRefs", reference.clone()));
    }
    if !configuration.is_empty() {
        command.env("GIT_CONFIG_COUNT", configuration.len().to_string());
        for (index, (key, value)) in configuration.iter().enumerate() {
            command
                .env(format!("GIT_CONFIG_KEY_{index}"), key)
                .env(format!("GIT_CONFIG_VALUE_{index}"), value);
        }
    }
    for (name, value) in [
        (
            "HEPH_RUNTIME_RECEIVE_CONTEXT_FILE",
            environment
                .runtime_receive_context_file
                .map(Path::as_os_str)
                .and_then(std::ffi::OsStr::to_str),
        ),
        (
            "HEPH_RUNTIME_RECEIVE_REPOSITORY",
            environment.runtime_receive_repository,
        ),
        (
            "HEPH_RUNTIME_RECEIVE_REQUEST_BYTES",
            environment.runtime_receive_request_bytes,
        ),
    ] {
        if let Some(value) = value {
            command.env(name, value);
        }
    }
    command
}

pub fn materialize_receive_context(
    context: &receive_policy::ResolvedRuntimeReceiveContext,
) -> Result<tempfile::NamedTempFile, GitHttpError> {
    let bytes = context.to_hook_json().map_err(domain)?;
    let mut file = tempfile::Builder::new()
        .prefix("hephaestus-runtime-git-")
        .suffix(".context")
        .tempfile()
        .map_err(GitHttpError::Io)?;
    std::io::Write::write_all(file.as_file_mut(), &bytes).map_err(GitHttpError::Io)?;
    file.as_file().sync_all().map_err(GitHttpError::Io)?;
    Ok(file)
}

pub async fn hidden_runtime_refs(
    repository: PathBuf,
    scope: &git_capability_domain::GitCapabilityScope,
    operation: git_capability_domain::GitOperation,
) -> Result<Vec<String>, GitHttpError> {
    Ok(snapshot_refs(repository)
        .await?
        .into_keys()
        .filter(|reference| !scope.allows(operation, reference.as_str()))
        .map(|reference| reference.as_str().to_owned())
        .collect())
}

pub async fn read_cgi_headers(
    stdout: &mut ChildStdout,
) -> Result<(StatusCode, HeaderMap, Bytes), GitHttpError> {
    let mut buffer = BytesMut::with_capacity(1024);
    loop {
        if let Some((end, delimiter_length)) = find_header_end(&buffer) {
            let raw = buffer.split_to(end);
            let _delimiter = buffer.split_to(delimiter_length);
            let (status, headers) = parse_cgi_headers(&raw)?;
            return Ok((status, headers, buffer.freeze()));
        }
        if buffer.len() >= MAX_CGI_HEADERS {
            return Err(GitHttpError::InvalidBackendResponse(
                "CGI headers exceed configured limit",
            ));
        }
        let read = stdout
            .read_buf(&mut buffer)
            .await
            .map_err(GitHttpError::Io)?;
        if read == 0 {
            return Err(GitHttpError::InvalidBackendResponse(
                "CGI response ended before its headers",
            ));
        }
    }
}

fn find_header_end(bytes: &[u8]) -> Option<(usize, usize)> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| (position, 4))
        .or_else(|| {
            bytes
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|position| (position, 2))
        })
}

pub fn parse_cgi_headers(bytes: &[u8]) -> Result<(StatusCode, HeaderMap), GitHttpError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| GitHttpError::InvalidBackendResponse("CGI headers are not UTF-8"))?;
    let mut status = StatusCode::OK;
    let mut headers = HeaderMap::new();
    for line in text.lines() {
        let (name, value) = line
            .split_once(':')
            .ok_or(GitHttpError::InvalidBackendResponse("malformed CGI header"))?;
        if name.eq_ignore_ascii_case("status") {
            let code = value
                .trim()
                .split_once(' ')
                .map_or_else(|| value.trim(), |(code, _)| code);
            status = StatusCode::from_bytes(code.as_bytes())
                .map_err(|_| GitHttpError::InvalidBackendResponse("invalid CGI status"))?;
            continue;
        }
        let name = HeaderName::from_bytes(name.trim().as_bytes())
            .map_err(|_| GitHttpError::InvalidBackendResponse("invalid CGI header name"))?;
        let value = HeaderValue::from_str(value.trim())
            .map_err(|_| GitHttpError::InvalidBackendResponse("invalid CGI header value"))?;
        headers.append(name, value);
    }
    Ok((status, headers))
}

pub async fn stream_response(
    mut stdout: ChildStdout,
    sender: mpsc::Sender<Result<Bytes, io::Error>>,
    remainder: Bytes,
    maximum: u64,
) -> io::Result<()> {
    let mut total = u64::try_from(remainder.len()).unwrap_or(u64::MAX);
    if total > maximum {
        let _ = sender
            .send(Err(io::Error::other(
                "Git response exceeds configured limit",
            )))
            .await;
        return Err(io::Error::other("Git response exceeds configured limit"));
    }
    if !remainder.is_empty() && sender.send(Ok(remainder)).await.is_err() {
        return Ok(());
    }
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = stdout.read(&mut buffer).await?;
        if read == 0 {
            return Ok(());
        }
        total = total
            .checked_add(u64::try_from(read).unwrap_or(u64::MAX))
            .ok_or_else(|| io::Error::other("Git response length overflow"))?;
        if total > maximum {
            let _ = sender
                .send(Err(io::Error::other(
                    "Git response exceeds configured limit",
                )))
                .await;
            return Err(io::Error::other("Git response exceeds configured limit"));
        }
        if sender
            .send(Ok(Bytes::copy_from_slice(&buffer[..read])))
            .await
            .is_err()
        {
            return Ok(());
        }
    }
}
