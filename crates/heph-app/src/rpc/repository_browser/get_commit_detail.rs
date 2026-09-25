use super::{RepositoryBrowserRpc, branch, commit_detail, map_error, parse_id};
use crate::application::repository_browser::CommitDetailRequest;
use crate::rpc::{RpcError, into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::{
    common::v1::PageResponse,
    repository_browser::v1::{GetCommitDetailRequest, GetCommitDetailResponse},
};

const DEFAULT_SIZE: u32 = 20;
const MAX_SIZE: usize = 50;

pub(super) async fn handle(
    service: &RepositoryBrowserRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, GetCommitDetailRequest>,
) -> ServiceResult<GetCommitDetailResponse> {
    let identity = request::query_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.repository_browser.v1.RepositoryBrowserService/GetCommitDetail",
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
    let skip = cursor.as_ref().map_or(0, |(_, _, offset)| *offset);
    let (selected, detail, has_more) = service
        .application
        .commit_detail(
            &identity,
            id,
            CommitDetailRequest {
                branch: &request.branch,
                commit: &request.commit,
                parent: &request.parent,
                skip,
                limit: size,
            },
        )
        .await
        .map_err(map_error)
        .map_err(into_connect_error)?;
    if cursor.as_ref().is_some_and(|(commit, parent, _)| {
        commit != &request.commit || parent != &parent_cursor_component(&detail.selected_parent)
    }) {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    let next_page_token = if has_more {
        format!(
            "v1:{}:{}:{}",
            request.commit,
            parent_cursor_component(&detail.selected_parent),
            skip + size
        )
    } else {
        String::new()
    };
    Response::ok(GetCommitDetailResponse {
        selected_branch: branch(selected).into(),
        detail: commit_detail(detail).into(),
        page: PageResponse {
            next_page_token,
            stable_order: String::from("path"),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}

fn parse_cursor(value: &str) -> Result<(String, String, usize), RpcError> {
    let mut parts = value.split(':');
    if parts.next() != Some("v1") {
        return Err(RpcError::InvalidArgument);
    }
    let commit = parts.next().ok_or(RpcError::InvalidArgument)?;
    let parent = parts.next().ok_or(RpcError::InvalidArgument)?;
    let offset = parts.next().ok_or(RpcError::InvalidArgument)?;
    if parts.next().is_some()
        || !valid_object_id(commit)
        || (parent != "root" && !valid_object_id(parent))
    {
        return Err(RpcError::InvalidArgument);
    }
    Ok((
        commit.to_owned(),
        parent.to_owned(),
        offset.parse().map_err(|_| RpcError::InvalidArgument)?,
    ))
}

fn valid_object_id(value: &str) -> bool {
    (value.len() == 40 || value.len() == 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn parent_cursor_component(parent: &str) -> String {
    if parent.is_empty() {
        String::from("root")
    } else {
        parent.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::parse_cursor;

    #[test]
    fn commit_diff_cursor_binds_commit_parent_and_offset() {
        let sha = "a".repeat(40);
        assert_eq!(
            parse_cursor(&format!("v1:{sha}:root:20")).expect("valid cursor"),
            (sha, String::from("root"), 20)
        );
        assert!(parse_cursor("v1:short:root:0").is_err());
    }
}
