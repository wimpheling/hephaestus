pub(super) async fn handle(
    service: &super::ReleaseRpc,
    ctx: connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<
        '_,
        rpc_proto::messages::hephaestus::release::v1::ListRepositoryReleasesRequest,
    >,
) -> connectrpc::ServiceResult<
    rpc_proto::messages::hephaestus::release::v1::ListRepositoryReleasesResponse,
> {
    use crate::application::release::{ReleasePage, decode_cursor, encode_cursor};
    use rpc_proto::messages::hephaestus::{
        common::v1::PageResponse, release::v1::ListRepositoryReleasesResponse,
    };
    use uuid::Uuid;

    use super::super::{RpcError, into_connect_error, request as shared_request};

    const AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/ListRepositoryReleases";
    const DEFAULT_PAGE_SIZE: u32 = 50;
    const MAX_PAGE_SIZE: u32 = 100;

    let identity = shared_request::query_identity(&ctx, &service.authenticator, AUDIENCE)
        .map_err(into_connect_error)?;
    let request = request.to_owned_message();
    let repository_id = shared_request::required_id(request.repository_id.as_option())
        .and_then(|value| Uuid::parse_str(&value).map_err(|_| RpcError::InvalidArgument))
        .map_err(into_connect_error)?;
    let page_size = request.page.as_option().map_or(DEFAULT_PAGE_SIZE, |page| {
        if page.page_size == 0 {
            DEFAULT_PAGE_SIZE
        } else {
            page.page_size
        }
    });
    if page_size > MAX_PAGE_SIZE {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    let after = request
        .page
        .as_option()
        .filter(|page| !page.page_token.is_empty())
        .map(|page| decode_cursor(&page.page_token).ok_or(RpcError::InvalidArgument))
        .transpose()
        .map_err(into_connect_error)?;
    let result = service
        .application
        .list_repository_releases(
            &identity,
            repository_id,
            ReleasePage {
                size: i64::from(page_size),
                after,
            },
        )
        .await
        .map_err(super::model::application_error)
        .map_err(into_connect_error)?;
    connectrpc::Response::ok(ListRepositoryReleasesResponse {
        releases: result
            .releases
            .into_iter()
            .map(super::model::summary)
            .collect(),
        page: PageResponse {
            next_page_token: result.next.map(encode_cursor).unwrap_or_default(),
            stable_order: String::from("created_at desc,id desc"),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}
