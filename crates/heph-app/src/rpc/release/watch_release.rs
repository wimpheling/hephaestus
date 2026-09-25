use super::ReleaseRpc;
use crate::{
    application::event::{EventScope, ScopeKind},
    rpc::{
        RpcError,
        event::{self, model::Delivery},
        into_connect_error, request,
    },
};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult, ServiceStream};
use futures_util::stream;
use rpc_proto::messages::hephaestus::{
    common::v1::Cursor,
    event::v1::{AggregateType, ProductEvent, product_event},
    release::v1::{
        ReleaseChange, WatchReleaseRequest, WatchReleaseResponse, watch_release_response,
    },
};
use std::sync::Arc;
use uuid::Uuid;

const AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/WatchRelease";

pub(super) async fn handle(
    service: &ReleaseRpc,
    ctx: RequestContext,
    request_message: ServiceRequest<'_, WatchReleaseRequest>,
) -> ServiceResult<ServiceStream<WatchReleaseResponse>> {
    let identity = request::query_identity(&ctx, &service.authenticator, AUDIENCE)
        .map_err(into_connect_error)?;
    let request = request_message.to_owned_message();
    let release_id = request::required_id(request.release_id.as_option())
        .and_then(|value| Uuid::parse_str(&value).map_err(|_| RpcError::InvalidArgument))
        .map_err(into_connect_error)?;
    let detail = service
        .application
        .get_release(&identity, release_id)
        .await
        .map_err(super::model::application_error)
        .map_err(into_connect_error)?;
    let repository_id = detail.repository_id;
    let receiver = event::watch::start_filtered(
        service.event_application.clone(),
        identity,
        EventScope {
            kind: ScopeKind::Repository,
            id: repository_id,
        },
        request
            .resume_cursor
            .as_option()
            .map(|cursor| cursor.value.as_str()),
        request.max_events,
        request.max_total_bytes,
        service.cursor_codec.clone(),
        Arc::new(move |event| is_release_event(event, release_id)),
    )
    .await?;
    let responses = stream::unfold(receiver, |mut receiver| async move {
        let result = receiver.recv().await?.and_then(response);
        Some((result, receiver))
    });
    Response::ok(Box::pin(responses))
}

fn response(frame: event::watch::Frame) -> Result<WatchReleaseResponse, connectrpc::ConnectError> {
    let item = match frame.delivery {
        Delivery::Barrier(value) => watch_release_response::Item::SnapshotBarrier(Box::new(value)),
        Delivery::Event(value) => watch_release_response::Item::Event(Box::new(
            release_change(&value).map_err(into_connect_error)?,
        )),
        Delivery::Gap(value) => watch_release_response::Item::RetentionGap(Box::new(value)),
        Delivery::Revoked(value) => watch_release_response::Item::AccessRevoked(Box::new(value)),
    };
    Ok(WatchReleaseResponse {
        sequence: frame.sequence,
        committed_cursor: Cursor {
            value: frame.committed_cursor,
            ..Default::default()
        }
        .into(),
        item: Some(item),
        ..Default::default()
    })
}

fn is_release_event(event: &ProductEvent, release_id: Uuid) -> bool {
    event.aggregate_type.as_known() == Some(AggregateType::Release)
        && event
            .aggregate_id
            .as_option()
            .is_some_and(|id| id.value == release_id.to_string())
        && matches!(
            event.payload.as_ref(),
            Some(product_event::Payload::ReleaseChanged(_))
        )
}

fn release_change(event: &ProductEvent) -> Result<ReleaseChange, RpcError> {
    let Some(product_event::Payload::ReleaseChanged(payload)) = event.payload.as_ref() else {
        return Err(RpcError::Internal);
    };
    Ok(ReleaseChange {
        event_id: event.event_id.as_option().cloned().into(),
        cursor: event.cursor.as_option().cloned().into(),
        release_id: event.aggregate_id.as_option().cloned().into(),
        repository_id: payload.repository_id.as_option().cloned().into(),
        aggregate_version: event.aggregate_version,
        change: payload.change,
        state: payload.state,
        occurred_at: event.occurred_at.as_option().cloned().into(),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::is_release_event;
    use rpc_proto::messages::hephaestus::common::v1::OpaqueId;
    use rpc_proto::messages::hephaestus::event::v1::{
        AggregateType, ChangeKind, LifecycleState, ProductEvent, ReleaseChanged, product_event,
    };
    use uuid::Uuid;

    #[test]
    fn release_watch_filters_to_the_requested_release() {
        let release_id = Uuid::new_v4();
        let event = ProductEvent {
            aggregate_type: AggregateType::Release.into(),
            aggregate_id: OpaqueId {
                value: release_id.to_string(),
                ..Default::default()
            }
            .into(),
            payload: Some(product_event::Payload::ReleaseChanged(Box::new(
                ReleaseChanged {
                    change: ChangeKind::Updated.into(),
                    state: LifecycleState::Published.into(),
                    ..Default::default()
                },
            ))),
            ..Default::default()
        };
        assert!(is_release_event(&event, release_id));
        assert!(!is_release_event(&event, Uuid::new_v4()));
    }
}
