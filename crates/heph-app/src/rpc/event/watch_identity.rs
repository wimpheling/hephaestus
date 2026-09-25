use super::{EventRpc, model::Delivery, watch};
use crate::{
    application::event::{EventScope, ScopeKind},
    rpc::{into_connect_error, request},
};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult, ServiceStream};
use futures_util::stream;
use rpc_proto::messages::hephaestus::{
    common::v1::Cursor,
    event::v1::{WatchIdentityRequest, WatchIdentityResponse, watch_identity_response},
};

const AUDIENCE: &str = "/hephaestus.event.v1.ProductEventService/WatchIdentity";

pub(super) async fn handle(
    service: &EventRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, WatchIdentityRequest>,
) -> ServiceResult<ServiceStream<WatchIdentityResponse>> {
    let identity = request::query_identity(&ctx, &service.authenticator, AUDIENCE)
        .map_err(into_connect_error)?;
    let request = request.to_owned_message();
    let scope = EventScope {
        kind: ScopeKind::Identity,
        id: identity.user_id.as_uuid(),
    };
    let receiver = watch::start(
        service.application.clone(),
        identity,
        scope,
        request
            .resume_cursor
            .as_option()
            .map(|cursor| cursor.value.as_str()),
        request.max_events,
        request.max_total_bytes,
        service.cursor_codec.clone(),
    )
    .await?;
    let responses = stream::unfold(receiver, |mut receiver| async move {
        let result = receiver.recv().await?.map(|frame| WatchIdentityResponse {
            sequence: frame.sequence,
            committed_cursor: Cursor {
                value: frame.committed_cursor.to_string(),
                ..Default::default()
            }
            .into(),
            item: Some(match frame.delivery {
                Delivery::Barrier(value) => {
                    watch_identity_response::Item::SnapshotBarrier(Box::new(value))
                }
                Delivery::Event(value) => watch_identity_response::Item::Event(Box::new(value)),
                Delivery::Gap(value) => {
                    watch_identity_response::Item::RetentionGap(Box::new(value))
                }
                Delivery::Revoked(value) => {
                    watch_identity_response::Item::AccessRevoked(Box::new(value))
                }
            }),
            ..Default::default()
        });
        Some((result, receiver))
    });
    Response::ok(Box::pin(responses))
}
