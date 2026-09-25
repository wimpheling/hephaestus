use super::BuildRpc;
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
    build::v1::{
        BuildChange, WatchRepositoryBuildsRequest, WatchRepositoryBuildsResponse,
        watch_repository_builds_response,
    },
    common::v1::Cursor,
    event::v1::{AggregateType, ProductEvent, product_event},
};
use std::sync::Arc;
use uuid::Uuid;

const AUDIENCE: &str = "/hephaestus.build.v1.BuildService/WatchRepositoryBuilds";

pub(super) async fn handle(
    service: &BuildRpc,
    ctx: RequestContext,
    request_message: ServiceRequest<'_, WatchRepositoryBuildsRequest>,
) -> ServiceResult<ServiceStream<WatchRepositoryBuildsResponse>> {
    let identity = request::query_identity(&ctx, &service.authenticator, AUDIENCE)
        .map_err(into_connect_error)?;
    let request = request_message.to_owned_message();
    let repository_id = request::required_id(request.repository_id.as_option())
        .and_then(|value| Uuid::parse_str(&value).map_err(|_| RpcError::InvalidArgument))
        .map_err(into_connect_error)?;
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
        Arc::new(is_build_event),
    )
    .await?;
    let responses = stream::unfold(receiver, |mut receiver| async move {
        let result = receiver.recv().await?.and_then(response);
        Some((result, receiver))
    });
    Response::ok(Box::pin(responses))
}

fn response(
    frame: event::watch::Frame,
) -> Result<WatchRepositoryBuildsResponse, connectrpc::ConnectError> {
    let item = match frame.delivery {
        Delivery::Barrier(value) => {
            watch_repository_builds_response::Item::SnapshotBarrier(Box::new(value))
        }
        Delivery::Event(value) => watch_repository_builds_response::Item::Event(Box::new(
            build_change(&value).map_err(into_connect_error)?,
        )),
        Delivery::Gap(value) => {
            watch_repository_builds_response::Item::RetentionGap(Box::new(value))
        }
        Delivery::Revoked(value) => {
            watch_repository_builds_response::Item::AccessRevoked(Box::new(value))
        }
    };
    Ok(WatchRepositoryBuildsResponse {
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

fn is_build_event(event: &ProductEvent) -> bool {
    event.aggregate_type.as_known() == Some(AggregateType::Build)
        && matches!(
            event.payload.as_ref(),
            Some(product_event::Payload::BuildChanged(_))
        )
}

fn build_change(event: &ProductEvent) -> Result<BuildChange, RpcError> {
    let Some(product_event::Payload::BuildChanged(payload)) = event.payload.as_ref() else {
        return Err(RpcError::Internal);
    };
    Ok(BuildChange {
        event_id: event.event_id.as_option().cloned().into(),
        cursor: event.cursor.as_option().cloned().into(),
        build_id: event.aggregate_id.as_option().cloned().into(),
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
    use super::is_build_event;
    use rpc_proto::messages::hephaestus::event::v1::{
        AggregateType, BuildChanged, ChangeKind, LifecycleState, ProductEvent, product_event,
    };

    #[test]
    fn repository_build_watch_filters_to_build_aggregate_events() {
        let event = ProductEvent {
            aggregate_type: AggregateType::Build.into(),
            payload: Some(product_event::Payload::BuildChanged(Box::new(
                BuildChanged {
                    change: ChangeKind::Created.into(),
                    state: LifecycleState::Queued.into(),
                    ..Default::default()
                },
            ))),
            ..Default::default()
        };
        assert!(is_build_event(&event));
    }

    #[test]
    fn repository_build_watch_rejects_repository_events() {
        let event = ProductEvent::default();
        assert!(!is_build_event(&event));
    }
}
