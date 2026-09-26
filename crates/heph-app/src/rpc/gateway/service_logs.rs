//! Connect adapter for the authorized persistent service-log reader.

use super::GatewayRpc;
use super::auth::{id, query};
use super::conversions::timestamp;
use crate::rpc::{RpcError, into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use gateway_edge::{
    GatewayServiceLogReadRecord, GatewayServiceLogReadRequest, GatewayServiceLogReadScope,
};
use gateway_postgres::GatewayServiceLogReaderError;
use rpc_proto::messages::hephaestus::{
    common::v1::Cursor,
    gateway::v1::{
        GatewayServiceLogMetadata, GatewayServiceLogRecord, GatewayServiceLogScope,
        GatewayServiceLogStream, ListGatewayServiceLogsRequest, ListGatewayServiceLogsResponse,
    },
};
use vm_trait::LogStream;

pub(super) async fn handle(
    rpc: &GatewayRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, ListGatewayServiceLogsRequest>,
) -> ServiceResult<ListGatewayServiceLogsResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let identity = query(&ctx, &rpc.authenticator, "ListGatewayServiceLogs")?;
    let request = message.to_owned_message();
    let scope = read_scope(request.scope.as_option())?;
    let limit = u16::try_from(request.limit).map_err(|_| invalid())?;
    let after = request
        .after
        .as_option()
        .map(|cursor| rpc.service_log_cursor.decode(&cursor.value, scope))
        .transpose()
        .map_err(|_| invalid())?;
    let request = GatewayServiceLogReadRequest::new(scope, limit, after).map_err(|_| invalid())?;
    let page = request::run_with_budget(&budget, rpc.service_logs.get_page(&identity, request))
        .await
        .map_err(into_connect_error)?
        .map_err(map_error)
        .map_err(into_connect_error)?;

    let records = page
        .records
        .iter()
        .map(record)
        .collect::<Result<Vec<_>, _>>()
        .map_err(into_connect_error)?;
    let next_after = page.next_after.map(|cursor| Cursor {
        value: rpc.service_log_cursor.encode(cursor),
        ..Default::default()
    });
    Response::ok(ListGatewayServiceLogsResponse {
        metadata: Some(metadata(page.metadata)).into(),
        records,
        history_incomplete: page.history_incomplete,
        next_after: next_after.into(),
        ..Default::default()
    })
}

fn read_scope(
    value: Option<&GatewayServiceLogScope>,
) -> Result<GatewayServiceLogReadScope, connectrpc::ConnectError> {
    let value = value.ok_or_else(invalid)?;
    let fencing_token = i64::try_from(value.fencing_token).map_err(|_| invalid())?;
    GatewayServiceLogReadScope::new(
        id(value.project_id.as_option())?,
        id(value.gateway_id.as_option())?,
        id(value.revision_id.as_option())?,
        id(value.instance_id.as_option())?,
        fencing_token,
    )
    .map_err(|_| invalid())
}

fn record(value: &GatewayServiceLogReadRecord) -> Result<GatewayServiceLogRecord, RpcError> {
    let stream = match value.stream {
        LogStream::Stdout => GatewayServiceLogStream::Stdout,
        LogStream::Stderr => GatewayServiceLogStream::Stderr,
        _ => return Err(RpcError::Internal),
    };
    Ok(GatewayServiceLogRecord {
        sequence: value.sequence,
        stream: stream.into(),
        observed_at: timestamp(value.observed_at).into(),
        stored_at: timestamp(value.stored_at).into(),
        contents: value.bytes.clone(),
        ..Default::default()
    })
}

fn metadata(value: gateway_edge::GatewayServiceLogReadMetadata) -> GatewayServiceLogMetadata {
    GatewayServiceLogMetadata {
        epoch_present: value.epoch_present,
        acknowledged_through: value.acknowledged_through,
        retained_bytes: value.retained_bytes,
        retained_chunks: value.retained_chunks,
        producer_dropped_chunks: value.producer_dropped_chunks,
        producer_dropped_bytes: value.producer_dropped_bytes,
        provider_lagged_events: value.provider_lagged_events,
        storage_dropped_chunks: value.storage_dropped_chunks,
        storage_dropped_bytes: value.storage_dropped_bytes,
        evicted_chunks: value.evicted_chunks,
        evicted_bytes: value.evicted_bytes,
        earliest_retained_sequence: value.earliest_retained_sequence,
        ..Default::default()
    }
}

const fn map_error(error: GatewayServiceLogReaderError) -> RpcError {
    match error {
        GatewayServiceLogReaderError::Denied => RpcError::PermissionDenied,
        GatewayServiceLogReaderError::NotFound => RpcError::NotFound,
        GatewayServiceLogReaderError::InvalidArgument => RpcError::InvalidArgument,
        GatewayServiceLogReaderError::Unavailable => RpcError::Unavailable,
    }
}

fn invalid() -> connectrpc::ConnectError {
    into_connect_error(RpcError::InvalidArgument)
}

#[cfg(test)]
mod tests {
    use super::read_scope;
    use rpc_proto::messages::hephaestus::common::v1::OpaqueId;
    use rpc_proto::messages::hephaestus::gateway::v1::GatewayServiceLogScope;
    use uuid::Uuid;

    fn opaque(id: Uuid) -> OpaqueId {
        OpaqueId {
            value: id.to_string(),
            ..Default::default()
        }
    }

    fn valid_scope() -> GatewayServiceLogScope {
        GatewayServiceLogScope {
            project_id: opaque(Uuid::from_u128(1)).into(),
            gateway_id: opaque(Uuid::from_u128(2)).into(),
            revision_id: opaque(Uuid::from_u128(3)).into(),
            instance_id: opaque(Uuid::from_u128(4)).into(),
            fencing_token: 9,
            ..Default::default()
        }
    }

    #[test]
    fn scope_requires_all_ids_and_positive_signed_fence() {
        assert!(read_scope(Some(&valid_scope())).is_ok());
        assert!(read_scope(None).is_err());
        let mut missing = valid_scope();
        missing.instance_id = None.into();
        assert!(read_scope(Some(&missing)).is_err());
        let mut zero = valid_scope();
        zero.fencing_token = 0;
        assert!(read_scope(Some(&zero)).is_err());
        let mut too_large = valid_scope();
        too_large.fencing_token = u64::try_from(i64::MAX).unwrap() + 1;
        assert!(read_scope(Some(&too_large)).is_err());
    }
}
