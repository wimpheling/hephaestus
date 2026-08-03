use super::{
    BuilderCatalogRpc, COMPLETE_PROJECT_BUILDER_PREPARATION_AUDIENCE,
    CREATE_PROJECT_BUILDER_AUDIENCE, GET_PROJECT_BUILDER_AUDIENCE, LIST_PROJECT_BUILDERS_AUDIENCE,
    MAX_PAGE_SIZE, REQUEST_PROJECT_BUILDER_PREPARATION_AUDIENCE, authorize_project,
    map_project_builder_error, parse_uuid, to_project_builder,
};
use crate::rpc::{into_connect_error, request};
use builder_catalog_application::CreateProjectBuilderRequest as ApplicationCreateProjectBuilderRequest;
use builder_catalog_domain::{OciDigest, ProjectBuilderProvenance};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::builder::v1::{
    CompleteProjectBuilderPreparationRequest, CompleteProjectBuilderPreparationResponse,
    CreateProjectBuilderRequest, CreateProjectBuilderResponse, GetProjectBuilderRequest,
    GetProjectBuilderResponse, ListProjectBuildersRequest, ListProjectBuildersResponse,
    RequestProjectBuilderPreparationRequest, RequestProjectBuilderPreparationResponse,
};
use rpc_proto::messages::hephaestus::common::v1::PageResponse;

const DEFAULT_PAGE_SIZE: u32 = 50;

pub(super) async fn create(
    service: &BuilderCatalogRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, CreateProjectBuilderRequest>,
) -> ServiceResult<CreateProjectBuilderResponse> {
    let request = message.to_owned_message();
    let identity = request::mutation_identity(
        &ctx,
        &service.authenticator,
        CREATE_PROJECT_BUILDER_AUDIENCE,
        request.context.as_option(),
    )
    .map_err(into_connect_error)?;
    let project_id = parse_uuid(request.project_id.as_option()).map_err(into_connect_error)?;
    let source_repository_id =
        parse_uuid(request.source_repository_id.as_option()).map_err(into_connect_error)?;
    authorize_project(service, &identity, project_id)
        .await
        .map_err(into_connect_error)?;
    let builder = service
        .project_application
        .create_project_builder(ApplicationCreateProjectBuilderRequest {
            project_id,
            source_repository_id,
            key: request.key,
            display_name: request.display_name,
            source_revision: request.source_revision,
            dockerfile_path: request.dockerfile_path,
            context_path: request.context_path,
            context_digest: request.context_digest,
            approved_base_image: request.approved_base_image_reference,
        })
        .await
        .map_err(map_project_builder_error)
        .map_err(into_connect_error)?;
    Response::ok(CreateProjectBuilderResponse {
        builder: to_project_builder(builder).into(),
        ..Default::default()
    })
}

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
        .list_project_builders(project_id)
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
        builders: builders.into_iter().map(to_project_builder).collect(),
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
        .get_project_builder(project_id, builder_id)
        .await
        .map_err(map_project_builder_error)
        .map_err(into_connect_error)?;
    Response::ok(GetProjectBuilderResponse {
        builder: to_project_builder(builder).into(),
        ..Default::default()
    })
}

pub(super) async fn request_preparation(
    service: &BuilderCatalogRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, RequestProjectBuilderPreparationRequest>,
) -> ServiceResult<RequestProjectBuilderPreparationResponse> {
    let request = message.to_owned_message();
    let identity = request::mutation_identity(
        &ctx,
        &service.authenticator,
        REQUEST_PROJECT_BUILDER_PREPARATION_AUDIENCE,
        request.context.as_option(),
    )
    .map_err(into_connect_error)?;
    let project_id = parse_uuid(request.project_id.as_option()).map_err(into_connect_error)?;
    let builder_id = parse_uuid(request.builder_id.as_option())
        .map(builder_catalog_domain::ProjectBuilderId::from_uuid)
        .map_err(into_connect_error)?;
    authorize_project(service, &identity, project_id)
        .await
        .map_err(into_connect_error)?;
    let builder = service
        .project_application
        .begin_project_builder_preparation(project_id, builder_id)
        .await
        .map_err(map_project_builder_error)
        .map_err(into_connect_error)?;
    Response::ok(RequestProjectBuilderPreparationResponse {
        builder: to_project_builder(builder).into(),
        ..Default::default()
    })
}

pub(super) async fn complete_preparation(
    service: &BuilderCatalogRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, CompleteProjectBuilderPreparationRequest>,
) -> ServiceResult<CompleteProjectBuilderPreparationResponse> {
    let request = message.to_owned_message();
    let identity = request::mutation_identity(
        &ctx,
        &service.authenticator,
        COMPLETE_PROJECT_BUILDER_PREPARATION_AUDIENCE,
        request.context.as_option(),
    )
    .map_err(into_connect_error)?;
    let project_id = parse_uuid(request.project_id.as_option()).map_err(into_connect_error)?;
    let builder_id = parse_uuid(request.builder_id.as_option())
        .map(builder_catalog_domain::ProjectBuilderId::from_uuid)
        .map_err(into_connect_error)?;
    let provenance = request
        .provenance
        .as_option()
        .ok_or(crate::rpc::RpcError::InvalidArgument)
        .and_then(|provenance| {
            Ok(ProjectBuilderProvenance {
                source_revision: provenance.source_revision.clone(),
                context_digest: OciDigest::parse(provenance.context_digest.clone())
                    .map_err(|_| crate::rpc::RpcError::InvalidArgument)?,
                attestation_reference: provenance.attestation_reference.clone(),
                sbom_reference: provenance.sbom_reference.clone(),
            })
        })
        .map_err(into_connect_error)?;
    authorize_project(service, &identity, project_id)
        .await
        .map_err(into_connect_error)?;
    let builder = service
        .project_application
        .complete_project_builder(
            project_id,
            builder_id,
            request.oci_image_reference,
            request.oci_image_digest,
            provenance,
        )
        .await
        .map_err(map_project_builder_error)
        .map_err(into_connect_error)?;
    Response::ok(CompleteProjectBuilderPreparationResponse {
        builder: to_project_builder(builder).into(),
        ..Default::default()
    })
}
