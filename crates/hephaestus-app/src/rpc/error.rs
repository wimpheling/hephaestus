//! Stable, non-sensitive transport error mapping.

use connectrpc::{ConnectError, ErrorDetail as ConnectErrorDetail};
use rpc_proto::messages::hephaestus::common::v1::{ErrorCode as DetailCode, ErrorDetail};

/// Transport-neutral failure categories produced by RPC adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcError {
    /// The request did not carry a valid mediator assertion.
    Unauthenticated,
    /// A supplied request field was malformed or outside its bound.
    InvalidArgument,
    /// The authenticated user cannot perform the operation.
    PermissionDenied,
    /// The authorized resource does not exist.
    NotFound,
    /// The requested resource already exists.
    AlreadyExists,
    /// Current durable state does not permit the operation.
    FailedPrecondition,
    /// A configured request or response size bound was exceeded.
    ResourceExhausted,
    /// The request budget expired.
    DeadlineExceeded,
    /// The caller canceled the operation.
    Canceled,
    /// A required dependency is temporarily unavailable.
    Unavailable,
    /// An unexpected failure occurred without safe client-facing detail.
    Internal,
}

/// Converts one adapter failure into its stable Connect code and message.
#[must_use]
pub fn into_connect_error(error: RpcError) -> ConnectError {
    let (mut transport, code, reason, retryable) = match error {
        RpcError::Unauthenticated => (
            ConnectError::unauthenticated("authentication required"),
            DetailCode::Unauthenticated,
            "authentication_required",
            false,
        ),
        RpcError::InvalidArgument => (
            ConnectError::invalid_argument("request is invalid"),
            DetailCode::InvalidArgument,
            "invalid_argument",
            false,
        ),
        RpcError::PermissionDenied => (
            ConnectError::permission_denied("permission denied"),
            DetailCode::PermissionDenied,
            "permission_denied",
            false,
        ),
        RpcError::NotFound => (
            ConnectError::not_found("resource not found"),
            DetailCode::NotFound,
            "not_found",
            false,
        ),
        RpcError::AlreadyExists => (
            ConnectError::already_exists("resource already exists"),
            DetailCode::AlreadyExists,
            "already_exists",
            false,
        ),
        RpcError::FailedPrecondition => (
            ConnectError::failed_precondition("operation precondition failed"),
            DetailCode::LifecycleConflict,
            "failed_precondition",
            false,
        ),
        RpcError::ResourceExhausted => (
            ConnectError::resource_exhausted("request limit exceeded"),
            DetailCode::ResourceExhausted,
            "resource_exhausted",
            false,
        ),
        RpcError::DeadlineExceeded => (
            ConnectError::deadline_exceeded("request deadline exceeded"),
            DetailCode::DeadlineExceeded,
            "deadline_exceeded",
            true,
        ),
        RpcError::Canceled => (
            ConnectError::canceled("request canceled"),
            DetailCode::Cancelled,
            "cancelled",
            false,
        ),
        RpcError::Unavailable => (
            ConnectError::unavailable("service unavailable"),
            DetailCode::Unavailable,
            "unavailable",
            true,
        ),
        RpcError::Internal => (
            ConnectError::internal("internal service error"),
            DetailCode::Internal,
            "internal",
            false,
        ),
    };
    let detail = ErrorDetail {
        code: code.into(),
        reason: reason.to_owned(),
        retryable,
        ..Default::default()
    };
    transport.details.push(ConnectErrorDetail::from_message(
        "hephaestus.common.v1.ErrorDetail",
        &detail,
    ));
    transport
}

#[cfg(test)]
mod tests {
    use super::{RpcError, into_connect_error};
    use connectrpc::ErrorCode;

    #[test]
    fn maps_stable_error_codes_without_sensitive_context() {
        let cases = [
            (RpcError::Unauthenticated, ErrorCode::Unauthenticated),
            (RpcError::InvalidArgument, ErrorCode::InvalidArgument),
            (RpcError::PermissionDenied, ErrorCode::PermissionDenied),
            (RpcError::NotFound, ErrorCode::NotFound),
            (RpcError::AlreadyExists, ErrorCode::AlreadyExists),
            (RpcError::FailedPrecondition, ErrorCode::FailedPrecondition),
            (RpcError::ResourceExhausted, ErrorCode::ResourceExhausted),
            (RpcError::DeadlineExceeded, ErrorCode::DeadlineExceeded),
            (RpcError::Canceled, ErrorCode::Canceled),
            (RpcError::Unavailable, ErrorCode::Unavailable),
            (RpcError::Internal, ErrorCode::Internal),
        ];
        let sentinel = "database-password-sentinel";

        for (source, expected) in cases {
            let error = into_connect_error(source);
            assert_eq!(error.code, expected);
            assert_eq!(error.details.len(), 1);
            assert_eq!(
                error.details[0].type_url,
                "hephaestus.common.v1.ErrorDetail"
            );
            assert!(error.details[0].value.is_some());
            assert!(!error.to_string().contains(sentinel));
            assert!(!String::from_utf8_lossy(&error.to_json()).contains(sentinel));
        }
    }
}
