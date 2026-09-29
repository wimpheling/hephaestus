//! Bootstrap RPC for creating a durable browser session.

use super::IdentityRpc;
use crate::rpc::{RpcError, into_connect_error, mutation_receipt, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use identity_application::{
    CreateBrowserSession, CreateBrowserSessionError, VerifiedBrowserIdentity,
};
use identity_domain::{BrowserSessionSid, RequestId, mutation_idempotency_seed};
use rpc_proto::messages::hephaestus::{
    common::v1::OpaqueId,
    identity::v1::{CreateBrowserSessionRequest, CreateBrowserSessionResponse},
};
use std::str::FromStr;
use uuid::Uuid;

pub(super) const AUDIENCE: &str = "/hephaestus.identity.v1.IdentityService/CreateBrowserSession";
const MAX_ISSUER_BYTES: usize = 2_048;
const MAX_SUBJECT_BYTES: usize = 2_048;
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 256;

pub(super) async fn handle(
    service: &IdentityRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, CreateBrowserSessionRequest>,
) -> ServiceResult<CreateBrowserSessionResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let request = message.to_owned_message();
    validate(&request).map_err(into_connect_error)?;
    service
        .authenticator
        .authenticate_session_bootstrap(ctx.headers(), AUDIENCE, &request.issuer, &request.subject)
        .map_err(|_| into_connect_error(RpcError::Unauthenticated))?;
    let context = request
        .context
        .as_option()
        .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?;
    let request_id = RequestId::from_str(
        &context
            .request_id
            .as_option()
            .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?
            .value,
    )
    .map_err(|_| into_connect_error(RpcError::InvalidArgument))?;
    let sid = BrowserSessionSid::from_uuid(
        Uuid::from_slice(&request.sid)
            .map_err(|_| into_connect_error(RpcError::InvalidArgument))?,
    );
    let result = request::run_with_budget(
        &budget,
        service
            .browser_sessions
            .create_browser_session(CreateBrowserSession {
                request_id,
                idempotency_seed: mutation_idempotency_seed(AUDIENCE, &context.idempotency_key),
                verified: VerifiedBrowserIdentity {
                    issuer: request.issuer,
                    subject: request.subject,
                },
                sid,
            }),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(|error| map_error(&error))
    .map_err(into_connect_error)?;
    let metadata = result.metadata;
    let receipt = request::run_with_budget(
        &budget,
        mutation_receipt(
            &service.receipts,
            result.idempotency_id,
            metadata.user_id(),
            "identity_profile",
            "identity",
        ),
    )
    .await
    .map_err(into_connect_error)??;
    Response::ok(CreateBrowserSessionResponse {
        user_id: OpaqueId {
            value: metadata.user_id().to_string(),
            ..Default::default()
        }
        .into(),
        session_id: OpaqueId {
            value: metadata.id().as_uuid().to_string(),
            ..Default::default()
        }
        .into(),
        expires_at: timestamp(metadata.expires_at()).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}

fn validate(request: &CreateBrowserSessionRequest) -> Result<(), RpcError> {
    if !bounded(&request.issuer, MAX_ISSUER_BYTES)
        || !bounded(&request.subject, MAX_SUBJECT_BYTES)
        || request.sid.len() != 16
    {
        return Err(RpcError::InvalidArgument);
    }
    let context = request
        .context
        .as_option()
        .ok_or(RpcError::InvalidArgument)?;
    if !bounded(&context.idempotency_key, MAX_IDEMPOTENCY_KEY_BYTES)
        || context.request_id.as_option().is_none()
    {
        return Err(RpcError::InvalidArgument);
    }
    Ok(())
}

const fn bounded(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum
}

const fn map_error(error: &CreateBrowserSessionError) -> RpcError {
    match error {
        CreateBrowserSessionError::PermissionDenied => RpcError::PermissionDenied,
        CreateBrowserSessionError::IdempotencyConflict
        | CreateBrowserSessionError::InactiveReplay => RpcError::FailedPrecondition,
        CreateBrowserSessionError::Unavailable => RpcError::Unavailable,
    }
}

fn timestamp(value: time::OffsetDateTime) -> buffa_types::google::protobuf::Timestamp {
    buffa_types::google::protobuf::Timestamp {
        seconds: value.unix_timestamp(),
        nanos: value.nanosecond().cast_signed(),
        ..Default::default()
    }
}
