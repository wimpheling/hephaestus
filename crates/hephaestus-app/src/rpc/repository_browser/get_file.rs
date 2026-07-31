use super::{RepositoryBrowserRpc, language, map_error, parse_id, tree_entry};
use crate::rpc::{RpcError, into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::repository_browser::v1::{GetFileRequest, GetFileResponse};

const MAX_FILE_BYTES: usize = 1_048_576;

pub(super) async fn handle(
    service: &RepositoryBrowserRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, GetFileRequest>,
) -> ServiceResult<GetFileResponse> {
    let identity = request::query_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.repository_browser.v1.RepositoryBrowserService/GetFile",
    )
    .map_err(into_connect_error)?;
    let request = message.to_owned_message();
    let id = parse_id(request.repository_id.as_option()).map_err(into_connect_error)?;
    let (_, entry, contents) = service
        .application
        .blob(
            &identity,
            id,
            &request.branch,
            &request.path,
            MAX_FILE_BYTES,
        )
        .await
        .map_err(map_error)
        .map_err(into_connect_error)?;
    let contents =
        String::from_utf8(contents).map_err(|_| into_connect_error(RpcError::InvalidArgument))?;
    if contents.contains('\0') {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    Response::ok(GetFileResponse {
        entry: tree_entry(entry).map_err(into_connect_error)?.into(),
        utf8_contents: contents,
        language: String::from(language(&request.path)),
        ..Default::default()
    })
}
