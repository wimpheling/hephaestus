use super::{ProjectRpc, map_error, opaque, parse_id, parse_page};
use crate::rpc::{into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::{
    common::v1::PageResponse,
    project::v1::{
        ListProjectRepositoriesRequest, ListProjectRepositoriesResponse, ProjectRepository,
    },
};

pub(super) async fn handle(
    service: &ProjectRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, ListProjectRepositoriesRequest>,
) -> ServiceResult<ListProjectRepositoriesResponse> {
    let identity = request::query_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.project.v1.ProjectService/ListProjectRepositories",
    )
    .map_err(into_connect_error)?;
    let request = message.to_owned_message();
    let project_id = parse_id(request.project_id.as_option()).map_err(into_connect_error)?;
    let page = parse_page(request.page.as_option()).map_err(into_connect_error)?;
    let result = service
        .application
        .repositories(&identity, project_id, page)
        .await
        .map_err(map_error)
        .map_err(into_connect_error)?;
    Response::ok(ListProjectRepositoriesResponse {
        repositories: result
            .values
            .into_iter()
            .map(|row| ProjectRepository {
                id: opaque(row.id).into(),
                name: row.name,
                default_branch: row.default_branch,
                is_public: row.is_public,
                attachment_count: row.attachment_count,
                run_count: row.run_count,
                ..Default::default()
            })
            .collect(),
        page: PageResponse {
            next_page_token: result.next.unwrap_or_default(),
            stable_order: String::from("id"),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}
