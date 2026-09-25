use crate::{
    application::event::{EventScope as AppScope, ScopeSnapshot},
    event_cursor::EventCursorCodec,
    rpc::RpcError,
};
use rpc_proto::messages::hephaestus::event::v1::{
    AccessRevoked, ProductEvent, RetentionGap, ScopeSnapshotBarrier,
};
use time::OffsetDateTime;

#[path = "projection.rs"]
mod projection;
#[cfg(test)]
#[path = "model/tests.rs"]
mod tests;

pub(crate) use projection::event;

pub(crate) enum Delivery {
    Barrier(ScopeSnapshotBarrier),
    Event(ProductEvent),
    Gap(RetentionGap),
    Revoked(AccessRevoked),
}

pub(super) fn barrier(
    codec: &EventCursorCodec,
    scope: AppScope,
    snapshot: &ScopeSnapshot,
) -> Result<ScopeSnapshotBarrier, RpcError> {
    Ok(ScopeSnapshotBarrier {
        scope: projection::proto_scope(scope).into(),
        committed_cursor: projection::cursor(codec, scope, snapshot.committed_cursor)?.into(),
        aggregate_versions: snapshot
            .aggregate_versions
            .iter()
            .map(projection::version)
            .collect::<Result<Vec<_>, _>>()?,
        schema_version: 1,
        ..Default::default()
    })
}

pub(super) fn gap(
    codec: &EventCursorCodec,
    scope: AppScope,
    requested: i64,
    earliest: i64,
    latest: i64,
) -> Result<RetentionGap, RpcError> {
    Ok(RetentionGap {
        scope: projection::proto_scope(scope).into(),
        requested_cursor: projection::cursor(codec, scope, requested)?.into(),
        earliest_available_cursor: projection::cursor(codec, scope, earliest)?.into(),
        latest_committed_cursor: projection::cursor(codec, scope, latest)?.into(),
        ..Default::default()
    })
}

pub(super) fn revoked(scope: AppScope) -> AccessRevoked {
    AccessRevoked {
        scope: projection::proto_scope(scope).into(),
        observed_at: projection::timestamp(OffsetDateTime::now_utc()).into(),
        ..Default::default()
    }
}
