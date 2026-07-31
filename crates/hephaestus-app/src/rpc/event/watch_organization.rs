use super::{EventRpc, model::Delivery, watch};
use crate::{
    application::event::{EventScope, ScopeKind},
    rpc::{RpcError, into_connect_error, request},
};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult, ServiceStream};
use futures_util::stream;
use rpc_proto::messages::hephaestus::{
    common::v1::Cursor,
    event::v1::{WatchOrganizationRequest, WatchOrganizationResponse, watch_organization_response},
};
use uuid::Uuid;

const AUDIENCE: &str = "/hephaestus.event.v1.ProductEventService/WatchOrganization";

pub(super) async fn handle(
    service: &EventRpc,
    ctx: RequestContext,
    request_message: ServiceRequest<'_, WatchOrganizationRequest>,
) -> ServiceResult<ServiceStream<WatchOrganizationResponse>> {
    let identity = request::query_identity(&ctx, &service.authenticator, AUDIENCE)
        .map_err(into_connect_error)?;
    let message = request_message.to_owned_message();
    let id = request::required_id(message.organization_id.as_option())
        .and_then(|value| Uuid::parse_str(&value).map_err(|_| RpcError::InvalidArgument))
        .map_err(into_connect_error)?;
    let receiver = watch::start(
        service.application.clone(),
        identity,
        EventScope {
            kind: ScopeKind::Organization,
            id,
        },
        message
            .resume_cursor
            .as_option()
            .map(|cursor| cursor.value.as_str()),
        message.max_events,
        message.max_total_bytes,
        service.cursor_codec.clone(),
    )
    .await?;
    let responses = stream::unfold(receiver, |mut receiver| async move {
        let result = receiver
            .recv()
            .await?
            .map(|frame| WatchOrganizationResponse {
                sequence: frame.sequence,
                committed_cursor: Cursor {
                    value: frame.committed_cursor.to_string(),
                    ..Default::default()
                }
                .into(),
                item: Some(match frame.delivery {
                    Delivery::Barrier(value) => {
                        watch_organization_response::Item::SnapshotBarrier(Box::new(value))
                    }
                    Delivery::Event(value) => {
                        watch_organization_response::Item::Event(Box::new(value))
                    }
                    Delivery::Gap(value) => {
                        watch_organization_response::Item::RetentionGap(Box::new(value))
                    }
                    Delivery::Revoked(value) => {
                        watch_organization_response::Item::AccessRevoked(Box::new(value))
                    }
                }),
                ..Default::default()
            });
        Some((result, receiver))
    });
    Response::ok(Box::pin(responses))
}
