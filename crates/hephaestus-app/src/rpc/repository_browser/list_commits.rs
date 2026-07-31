use super::{RepositoryBrowserRpc, branch, map_error, parse_id, timestamp};
use crate::rpc::{RpcError, into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::{
    common::v1::PageResponse,
    repository_browser::v1::{Commit, ListCommitsRequest, ListCommitsResponse},
};

const DEFAULT_SIZE: u32 = 50;
const MAX_SIZE: usize = 100;

pub(super) async fn handle(
    service: &RepositoryBrowserRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, ListCommitsRequest>,
) -> ServiceResult<ListCommitsResponse> {
    let identity = request::query_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.repository_browser.v1.RepositoryBrowserService/ListCommits",
    )
    .map_err(into_connect_error)?;
    let request = message.to_owned_message();
    let id = parse_id(request.repository_id.as_option()).map_err(into_connect_error)?;
    let page = request.page.as_option();
    let size = usize::try_from(page.map_or(DEFAULT_SIZE, |page| {
        if page.page_size == 0 {
            DEFAULT_SIZE
        } else {
            page.page_size
        }
    }))
    .map_err(|_| into_connect_error(RpcError::InvalidArgument))?;
    if size > MAX_SIZE {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    let cursor = page
        .filter(|page| !page.page_token.is_empty())
        .map(|page| parse_cursor(&page.page_token))
        .transpose()
        .map_err(into_connect_error)?;
    let skip = cursor.as_ref().map_or(0, |(_, offset)| *offset);
    let (selected, mut values) = service
        .application
        .commits(&identity, id, &request.branch, skip, size + 1)
        .await
        .map_err(map_error)
        .map_err(into_connect_error)?;
    if cursor
        .as_ref()
        .is_some_and(|(anchor, _)| anchor != &selected.commit)
    {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    let has_more = values.len() > size;
    values.truncate(size);
    let commits = values
        .into_iter()
        .map(|value| {
            Ok(Commit {
                id: value.id,
                parents: value.parents,
                author_name: value.author_name,
                author_email: value.author_email,
                authored_at: timestamp(value.authored_at).into(),
                subject: value.subject,
                ..Default::default()
            })
        })
        .collect::<Result<Vec<_>, RpcError>>()
        .map_err(into_connect_error)?;
    let next = if has_more {
        format!("v1:{}:{}", selected.commit, skip + size)
    } else {
        String::new()
    };
    Response::ok(ListCommitsResponse {
        selected_branch: branch(selected).into(),
        commits,
        page: PageResponse {
            next_page_token: next,
            stable_order: String::from("topological,commit"),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}

fn parse_cursor(value: &str) -> Result<(String, usize), RpcError> {
    let value = value.strip_prefix("v1:").ok_or(RpcError::InvalidArgument)?;
    let (anchor, offset) = value.rsplit_once(':').ok_or(RpcError::InvalidArgument)?;
    if anchor.len() != 40 && anchor.len() != 64 {
        return Err(RpcError::InvalidArgument);
    }
    Ok((
        anchor.to_owned(),
        offset.parse().map_err(|_| RpcError::InvalidArgument)?,
    ))
}
