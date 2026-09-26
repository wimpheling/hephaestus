use super::model::{
    application_error, grant_summary, import_summary, page_response, parse_page, parse_uuid, query,
    secret_summary,
};
use crate::rpc::{into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::secret::v1::{
    GetProjectSecretAuthorityRequest, GetProjectSecretAuthorityResponse,
    ListOrganizationSecretGrantsRequest, ListOrganizationSecretGrantsResponse,
    ListOrganizationSecretsRequest, ListOrganizationSecretsResponse, ListProjectSecretsRequest,
    ListProjectSecretsResponse,
};

pub(super) async fn list_project_secrets(
    service: &super::SecretRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, ListProjectSecretsRequest>,
) -> ServiceResult<ListProjectSecretsResponse> {
    let identity = query(&ctx, &service.authenticator, "ListProjectSecrets")?;
    let request = request.to_owned_message();
    let budget = request::RequestBudget::from_transport(&ctx);
    let result = request::run_with_budget(
        &budget,
        service.application.list_project_secrets(
            &identity,
            parse_uuid(request.project_id.as_option())?,
            parse_page(request.page.as_option())?,
        ),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(application_error)?;
    Response::ok(ListProjectSecretsResponse {
        page: page_response(result.next_page_token, "name,id").into(),
        secrets: result.values.into_iter().map(secret_summary).collect(),
        ..Default::default()
    })
}

pub(super) async fn list_organization_secrets(
    service: &super::SecretRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, ListOrganizationSecretsRequest>,
) -> ServiceResult<ListOrganizationSecretsResponse> {
    let identity = query(&ctx, &service.authenticator, "ListOrganizationSecrets")?;
    let request = request.to_owned_message();
    let budget = request::RequestBudget::from_transport(&ctx);
    let result = request::run_with_budget(
        &budget,
        service.application.list_organization_secrets(
            &identity,
            parse_uuid(request.organization_id.as_option())?,
            parse_page(request.page.as_option())?,
        ),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(application_error)?;
    Response::ok(ListOrganizationSecretsResponse {
        page: page_response(result.next_page_token, "name,id").into(),
        secrets: result.values.into_iter().map(secret_summary).collect(),
        ..Default::default()
    })
}

pub(super) async fn list_organization_secret_grants(
    service: &super::SecretRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, ListOrganizationSecretGrantsRequest>,
) -> ServiceResult<ListOrganizationSecretGrantsResponse> {
    let identity = query(&ctx, &service.authenticator, "ListOrganizationSecretGrants")?;
    let request = request.to_owned_message();
    let budget = request::RequestBudget::from_transport(&ctx);
    let result = request::run_with_budget(
        &budget,
        service.application.list_organization_grants(
            &identity,
            parse_uuid(request.organization_id.as_option())?,
            parse_page(request.page.as_option())?,
        ),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(application_error)?;
    Response::ok(ListOrganizationSecretGrantsResponse {
        page: page_response(result.next_page_token, "secret_name,created_at,id").into(),
        grants: result.values.into_iter().map(grant_summary).collect(),
        ..Default::default()
    })
}

pub(super) async fn get_project_secret_authority(
    service: &super::SecretRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, GetProjectSecretAuthorityRequest>,
) -> ServiceResult<GetProjectSecretAuthorityResponse> {
    let identity = query(&ctx, &service.authenticator, "GetProjectSecretAuthority")?;
    let request = request.to_owned_message();
    let budget = request::RequestBudget::from_transport(&ctx);
    let authority = request::run_with_budget(
        &budget,
        service.application.project_authority(
            &identity,
            parse_uuid(request.project_id.as_option())?,
            parse_page(request.grants_page.as_option())?,
            parse_page(request.imports_page.as_option())?,
        ),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(application_error)?;
    let grants_page = page_response(authority.grants.next_page_token, "secret_name,id");
    let imports_page = page_response(authority.imports.next_page_token, "alias,id");
    Response::ok(GetProjectSecretAuthorityResponse {
        grants: authority
            .grants
            .values
            .into_iter()
            .map(grant_summary)
            .collect(),
        imports: authority
            .imports
            .values
            .into_iter()
            .map(import_summary)
            .collect(),
        grants_page: grants_page.into(),
        imports_page: imports_page.into(),
        ..Default::default()
    })
}
