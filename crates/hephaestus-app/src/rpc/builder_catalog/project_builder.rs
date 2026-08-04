use super::{
    BuilderCatalogRpc, GET_PROJECT_BUILDER_AUDIENCE, LIST_PROJECT_BUILDERS_AUDIENCE, MAX_PAGE_SIZE,
    authorize_project, map_project_builder_error, parse_uuid, to_project_builder_with_registry,
};
use crate::rpc::{into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::builder::v1::{
    GetProjectBuilderRequest, GetProjectBuilderResponse, ListProjectBuildersRequest,
    ListProjectBuildersResponse,
};
use rpc_proto::messages::hephaestus::common::v1::PageResponse;

const DEFAULT_PAGE_SIZE: u32 = 50;

pub(super) async fn list(
    service: &BuilderCatalogRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, ListProjectBuildersRequest>,
) -> ServiceResult<ListProjectBuildersResponse> {
    let identity =
        request::query_identity(&ctx, &service.authenticator, LIST_PROJECT_BUILDERS_AUDIENCE)
            .map_err(into_connect_error)?;
    let request = message.to_owned_message();
    let project_id = parse_uuid(request.project_id.as_option()).map_err(into_connect_error)?;
    authorize_project(service, &identity, project_id)
        .await
        .map_err(into_connect_error)?;
    let page = request.page.as_option();
    let page_size = page.map_or(DEFAULT_PAGE_SIZE, |page| {
        if page.page_size == 0 {
            DEFAULT_PAGE_SIZE
        } else {
            page.page_size
        }
    });
    if page_size > MAX_PAGE_SIZE {
        return Err(into_connect_error(crate::rpc::RpcError::InvalidArgument));
    }
    let offset = page
        .filter(|page| !page.page_token.is_empty())
        .map_or(Ok(0_usize), |page| {
            page.page_token
                .parse::<usize>()
                .map_err(|_| crate::rpc::RpcError::InvalidArgument)
        })
        .map_err(into_connect_error)?;
    let builders = service
        .project_application
        .list_project_builder_publications(&identity, project_id)
        .await
        .map_err(map_project_builder_error)
        .map_err(into_connect_error)?;
    let page_size = usize::try_from(page_size).expect("bounded page size fits usize");
    let mut builders = builders.into_iter().skip(offset).collect::<Vec<_>>();
    let has_more = builders.len() > page_size;
    if has_more {
        builders.truncate(page_size);
    }
    Response::ok(ListProjectBuildersResponse {
        builders: builders
            .into_iter()
            .map(|publication| {
                to_project_builder_with_registry(
                    publication.builder,
                    publication.registry_publication,
                )
            })
            .collect(),
        page: PageResponse {
            next_page_token: if has_more {
                (offset + page_size).to_string()
            } else {
                String::new()
            },
            stable_order: String::from("key,id"),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}

pub(super) async fn get(
    service: &BuilderCatalogRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, GetProjectBuilderRequest>,
) -> ServiceResult<GetProjectBuilderResponse> {
    let identity =
        request::query_identity(&ctx, &service.authenticator, GET_PROJECT_BUILDER_AUDIENCE)
            .map_err(into_connect_error)?;
    let request = message.to_owned_message();
    let project_id = parse_uuid(request.project_id.as_option()).map_err(into_connect_error)?;
    let builder_id = parse_uuid(request.builder_id.as_option())
        .map(builder_catalog_domain::ProjectBuilderId::from_uuid)
        .map_err(into_connect_error)?;
    authorize_project(service, &identity, project_id)
        .await
        .map_err(into_connect_error)?;
    let builder = service
        .project_application
        .get_project_builder_publication(&identity, project_id, builder_id)
        .await
        .map_err(map_project_builder_error)
        .map_err(into_connect_error)?;
    Response::ok(GetProjectBuilderResponse {
        builder: to_project_builder_with_registry(builder.builder, builder.registry_publication)
            .into(),
        ..Default::default()
    })
}
