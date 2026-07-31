use super::{RepositoryBrowserRpc, branch, map_error, parse_id};
use crate::rpc::{RpcError, into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::{
    common::v1::PageResponse,
    repository_browser::v1::{ListBranchesRequest, ListBranchesResponse},
};

const DEFAULT_SIZE: u32 = 50;
const MAX_SIZE: usize = 200;

pub(super) async fn handle(
    service: &RepositoryBrowserRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, ListBranchesRequest>,
) -> ServiceResult<ListBranchesResponse> {
    let identity = request::query_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.repository_browser.v1.RepositoryBrowserService/ListBranches",
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
    let after = page
        .map(|page| {
            page.page_token
                .strip_prefix("v1:")
                .ok_or(RpcError::InvalidArgument)
        })
        .transpose()
        .map_err(into_connect_error)?;
    let values = service
        .application
        .branches(&identity, id)
        .await
        .map_err(map_error)
        .map_err(into_connect_error)?;
    let mut filtered = values
        .into_iter()
        .filter(|value| after.is_none_or(|after| value.name.as_str() > after))
        .take(size + 1)
        .collect::<Vec<_>>();
    let has_more = filtered.len() > size;
    filtered.truncate(size);
    let next = has_more
        .then(|| filtered.last().map(|value| format!("v1:{}", value.name)))
        .flatten()
        .unwrap_or_default();
    let branches = filtered.into_iter().map(branch).collect();
    Response::ok(ListBranchesResponse {
        branches,
        page: PageResponse {
            next_page_token: next,
            stable_order: String::from("refname"),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}
