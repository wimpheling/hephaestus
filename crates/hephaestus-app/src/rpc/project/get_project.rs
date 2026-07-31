use super::{ProjectRpc, map_error, opaque, parse_id};
use crate::rpc::{into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::project::v1::{
    GetProjectRequest, GetProjectResponse, Project,
};

pub(super) async fn handle(
    service: &ProjectRpc,
    ctx: RequestContext,
    request_message: ServiceRequest<'_, GetProjectRequest>,
) -> ServiceResult<GetProjectResponse> {
    let identity = request::query_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.project.v1.ProjectService/GetProject",
    )
    .map_err(into_connect_error)?;
    let project_id = parse_id(request_message.to_owned_message().project_id.as_option())
        .map_err(into_connect_error)?;
    let row = service
        .application
        .get(&identity, project_id)
        .await
        .map_err(map_error)
        .map_err(into_connect_error)?;
    Response::ok(GetProjectResponse {
        project: Project {
            id: opaque(row.id).into(),
            name: row.name,
            organization_id: opaque(row.organization_id).into(),
            organization_name: row.organization_name,
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}
