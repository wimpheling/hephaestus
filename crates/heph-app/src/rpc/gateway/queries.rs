use super::GatewayRpc;
use super::auth::{id, page, query};
use super::conversions::{
    ingress, mailbox_binding, mailbox_publication, map_error, revision, summary,
};
use crate::rpc::{into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::common::v1::PageResponse;
use rpc_proto::messages::hephaestus::gateway::v1::{
    GetGatewayRequest, GetGatewayResponse, ListGatewayIngressRequest, ListGatewayIngressResponse,
    ListMailboxBindingsRequest, ListMailboxBindingsResponse, ListMailboxPublicationsRequest,
    ListMailboxPublicationsResponse, ListProjectGatewaysRequest, ListProjectGatewaysResponse,
};

pub(super) async fn list_project_gateways(
    service: &GatewayRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, ListProjectGatewaysRequest>,
) -> ServiceResult<ListProjectGatewaysResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let identity = query(&ctx, &service.authenticator, "ListProjectGateways")?;
    let request = message.to_owned_message();
    let values = request::run_with_budget(
        &budget,
        service.application.list_project(
            &identity,
            id(request.project_id.as_option())?,
            page(request.page.as_option())?,
        ),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(|error| map_error(&error))
    .map_err(into_connect_error)?;
    Response::ok(ListProjectGatewaysResponse {
        gateways: values.into_iter().map(summary).collect(),
        page: PageResponse {
            stable_order: String::from("id"),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}

pub(super) async fn get_gateway(
    service: &GatewayRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, GetGatewayRequest>,
) -> ServiceResult<GetGatewayResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let identity = query(&ctx, &service.authenticator, "GetGateway")?;
    let request = message.to_owned_message();
    let (gateway, revisions) = request::run_with_budget(
        &budget,
        service
            .application
            .get(&identity, id(request.gateway_id.as_option())?),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(|error| map_error(&error))
    .map_err(into_connect_error)?;
    Response::ok(GetGatewayResponse {
        gateway: summary(gateway).into(),
        revisions: revisions.into_iter().map(revision).collect(),
        page: PageResponse {
            stable_order: String::from("created_at_desc"),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}

pub(super) async fn list_gateway_ingress(
    service: &GatewayRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, ListGatewayIngressRequest>,
) -> ServiceResult<ListGatewayIngressResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let identity = query(&ctx, &service.authenticator, "ListGatewayIngress")?;
    let request = message.to_owned_message();
    let values = request::run_with_budget(
        &budget,
        service.application.ingress(
            &identity,
            id(request.gateway_id.as_option())?,
            page(request.page.as_option())?,
        ),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(|error| map_error(&error))
    .map_err(into_connect_error)?;
    Response::ok(ListGatewayIngressResponse {
        ingress: values.iter().map(ingress).collect(),
        page: PageResponse {
            stable_order: String::from("id_desc"),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}

pub(super) async fn list_mailbox_bindings(
    service: &GatewayRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, ListMailboxBindingsRequest>,
) -> ServiceResult<ListMailboxBindingsResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let identity = query(&ctx, &service.authenticator, "ListMailboxBindings")?;
    let request = message.to_owned_message();
    let bindings = request::run_with_budget(
        &budget,
        service.application.mailbox_bindings(
            &identity,
            id(request.gateway_revision_id.as_option())?,
            page(request.page.as_option())?,
        ),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(|error| map_error(&error))
    .map_err(into_connect_error)?;
    Response::ok(ListMailboxBindingsResponse {
        bindings: bindings.into_iter().map(mailbox_binding).collect(),
        page: PageResponse {
            stable_order: String::from("id"),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}

pub(super) async fn list_mailbox_publications(
    service: &GatewayRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, ListMailboxPublicationsRequest>,
) -> ServiceResult<ListMailboxPublicationsResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let identity = query(&ctx, &service.authenticator, "ListMailboxPublications")?;
    let request = message.to_owned_message();
    let publications = request::run_with_budget(
        &budget,
        service.application.mailbox_publications(
            &identity,
            id(request.gateway_id.as_option())?,
            page(request.page.as_option())?,
        ),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(|error| map_error(&error))
    .map_err(into_connect_error)?;
    Response::ok(ListMailboxPublicationsResponse {
        publications: publications.into_iter().map(mailbox_publication).collect(),
        page: PageResponse {
            stable_order: String::from("id_desc"),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}
