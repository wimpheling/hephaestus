//! Connect adapter for project-wide service-log loss metadata.

use super::{GatewayRpc, id, query};
use crate::rpc::{RpcError, into_connect_error};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use gateway_edge::GatewayServiceLogProjectMetadata as ReaderMetadata;
use gateway_postgres::GatewayServiceLogReaderError;
use rpc_proto::messages::hephaestus::gateway::v1::{
    GatewayServiceLogProjectMetadata, GetProjectServiceLogMetadataRequest,
    GetProjectServiceLogMetadataResponse,
};

pub(super) async fn handle(
    rpc: &GatewayRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, GetProjectServiceLogMetadataRequest>,
) -> ServiceResult<GetProjectServiceLogMetadataResponse> {
    let identity = query(&ctx, &rpc.authenticator, "GetProjectServiceLogMetadata")?;
    let request = message.to_owned_message();
    let metadata = rpc
        .service_logs
        .get_project_metadata(&identity, id(request.project_id.as_option())?)
        .await
        .map_err(map_error)
        .map_err(into_connect_error)?;
    Response::ok(GetProjectServiceLogMetadataResponse {
        metadata: Some(metadata_message(metadata)).into(),
        ..Default::default()
    })
}

fn metadata_message(value: ReaderMetadata) -> GatewayServiceLogProjectMetadata {
    GatewayServiceLogProjectMetadata {
        usage_present: value.usage_present,
        storage_dropped_chunks: value.storage_dropped_chunks,
        storage_dropped_bytes: value.storage_dropped_bytes,
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

#[cfg(test)]
mod tests {
    use super::metadata_message;
    use gateway_edge::GatewayServiceLogProjectMetadata as ReaderMetadata;

    #[test]
    fn metadata_conversion_preserves_presence_and_loss_counters() {
        let metadata = metadata_message(ReaderMetadata {
            usage_present: true,
            storage_dropped_chunks: 7,
            storage_dropped_bytes: 1234,
        });

        assert!(metadata.usage_present);
        assert_eq!(metadata.storage_dropped_chunks, 7);
        assert_eq!(metadata.storage_dropped_bytes, 1234);
    }
}
