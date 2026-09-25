use crate::application::run::RunError;
use crate::rpc::{RpcError, into_connect_error};

pub(super) fn map_error(error: RunError) -> connectrpc::ConnectError {
    match error {
        RunError::NotFound => into_connect_error(RpcError::NotFound),
        RunError::InvalidPage => into_connect_error(RpcError::InvalidArgument),
        RunError::IdempotencyConflict => into_connect_error(RpcError::FailedPrecondition),
        RunError::PreviewUnavailable => into_connect_error(RpcError::Unavailable),
        RunError::Persistence(source) => {
            tracing::error!(error=%source,"run operation failed");
            into_connect_error(RpcError::Unavailable)
        }
    }
}

pub(super) fn invalid<T>(_error: T) -> connectrpc::ConnectError {
    into_connect_error(RpcError::InvalidArgument)
}
