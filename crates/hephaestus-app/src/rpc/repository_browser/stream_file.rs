use super::{RepositoryBrowserRpc, map_error, parse_id};
use crate::rpc::{RpcError, into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult, ServiceStream};
use futures_util::stream;
use rpc_proto::messages::hephaestus::{
    common::v1::Cursor,
    repository_browser::v1::{StreamFileRequest, StreamFileResponse},
};

const DEFAULT_TOTAL: usize = 16 * 1_048_576;
const MAX_TOTAL: usize = 16 * 1_048_576;
const DEFAULT_CHUNK: usize = 64 * 1_024;
const MAX_CHUNK: usize = 1_048_576;

pub(super) async fn handle(
    service: &RepositoryBrowserRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, StreamFileRequest>,
) -> ServiceResult<ServiceStream<StreamFileResponse>> {
    let identity = request::query_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.repository_browser.v1.RepositoryBrowserService/StreamFile",
    )
    .map_err(into_connect_error)?;
    let request = message.to_owned_message();
    let id = parse_id(request.repository_id.as_option()).map_err(into_connect_error)?;
    let total = if request.max_total_bytes == 0 {
        DEFAULT_TOTAL
    } else {
        usize::try_from(request.max_total_bytes)
            .map_err(|_| into_connect_error(RpcError::InvalidArgument))?
    };
    let chunk = if request.max_chunk_bytes == 0 {
        DEFAULT_CHUNK
    } else {
        usize::try_from(request.max_chunk_bytes)
            .map_err(|_| into_connect_error(RpcError::InvalidArgument))?
    };
    if total > MAX_TOTAL || chunk == 0 || chunk > MAX_CHUNK {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    let (selected, entry, contents) = service
        .application
        .blob(&identity, id, &request.branch, &request.path, MAX_TOTAL)
        .await
        .map_err(map_error)
        .map_err(into_connect_error)?;
    let offset = parse_cursor(
        request
            .resume_cursor
            .as_option()
            .map(|cursor| cursor.value.as_str()),
        &selected.commit,
        &entry.object_id,
        contents.len(),
    )
    .map_err(into_connect_error)?;
    let end = offset.saturating_add(total).min(contents.len());
    let media_type = media_type(&request.path);
    let mut responses = contents[offset..end]
        .chunks(chunk)
        .enumerate()
        .map(|(index, bytes)| {
            let committed = offset + ((index + 1) * chunk).min(end - offset);
            Ok(StreamFileResponse {
                sequence: u64::try_from(index).unwrap_or(u64::MAX),
                contents: bytes.to_vec(),
                committed_cursor: Cursor {
                    value: format!("v1:{}:{}:{committed}", selected.commit, entry.object_id),
                    ..Default::default()
                }
                .into(),
                end_of_file: committed == contents.len(),
                media_type: String::from(media_type),
                ..Default::default()
            })
        })
        .collect::<Vec<Result<_, connectrpc::ConnectError>>>();
    if responses.is_empty() && end == contents.len() {
        responses.push(Ok(StreamFileResponse {
            committed_cursor: Cursor {
                value: format!("v1:{}:{}:{end}", selected.commit, entry.object_id),
                ..Default::default()
            }
            .into(),
            end_of_file: true,
            media_type: String::from(media_type),
            ..Default::default()
        }));
    }
    Response::stream_ok(stream::iter(responses))
}

fn parse_cursor(
    value: Option<&str>,
    commit: &str,
    object: &str,
    length: usize,
) -> Result<usize, RpcError> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok(0);
    };
    let value = value.strip_prefix("v1:").ok_or(RpcError::InvalidArgument)?;
    let mut fields = value.split(':');
    let cursor_commit = fields.next().ok_or(RpcError::InvalidArgument)?;
    let cursor_object = fields.next().ok_or(RpcError::InvalidArgument)?;
    let offset: usize = fields
        .next()
        .ok_or(RpcError::InvalidArgument)?
        .parse()
        .map_err(|_| RpcError::InvalidArgument)?;
    if fields.next().is_some()
        || cursor_commit != commit
        || cursor_object != object
        || offset > length
    {
        return Err(RpcError::InvalidArgument);
    }
    Ok(offset)
}

fn media_type(path: &str) -> &'static str {
    match std::path::Path::new(path)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
    {
        Some("json") => "application/json",
        Some("md" | "txt" | "rs" | "ex" | "exs" | "toml" | "sql" | "sh" | "yml" | "yaml") => {
            "text/plain; charset=utf-8"
        }
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::parse_cursor;
    #[test]
    fn cursor_is_bound_to_commit_object_and_length() {
        assert_eq!(
            parse_cursor(Some("v1:abc:def:4"), "abc", "def", 8).expect("cursor"),
            4
        );
        assert!(parse_cursor(Some("v1:other:def:4"), "abc", "def", 8).is_err());
        assert!(parse_cursor(Some("v1:abc:def:9"), "abc", "def", 8).is_err());
    }
}
