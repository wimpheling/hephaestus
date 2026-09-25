//! Mediator RPC for revoking the current browser session.

use super::IdentityRpc;
use crate::rpc::{RpcError, into_connect_error, mutation_receipt, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use identity_application::{RevokeBrowserSession, RevokeBrowserSessionError};
use identity_domain::mutation_idempotency_seed;
use rpc_proto::messages::hephaestus::identity::v1::{
    RevokeBrowserSessionRequest, RevokeBrowserSessionResponse,
};

pub(super) const AUDIENCE: &str = "/hephaestus.identity.v1.IdentityService/RevokeBrowserSession";

pub(super) async fn handle(
    service: &IdentityRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, RevokeBrowserSessionRequest>,
) -> ServiceResult<RevokeBrowserSessionResponse> {
    let request = message.to_owned_message();
    let identity = request::mutation_identity(
        &ctx,
        &service.authenticator,
        AUDIENCE,
        request.context.as_option(),
    )
    .map_err(into_connect_error)?;
    let session = request::verified_mediator_session(&ctx).map_err(into_connect_error)?;
    if session.user_id != identity.user_id {
        return Err(into_connect_error(RpcError::PermissionDenied));
    }
    let context = request
        .context
        .as_option()
        .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let result = request::run_with_budget(
        &budget,
        service
            .browser_sessions
            .revoke_browser_session(RevokeBrowserSession {
                request_id: identity.request_id,
                idempotency_seed: mutation_idempotency_seed(AUDIENCE, &context.idempotency_key),
                user_id: identity.user_id,
                sid: session.sid,
            }),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(|error| map_error(&error))
    .map_err(into_connect_error)?;
    let receipt = request::run_with_budget(
        &budget,
        mutation_receipt(
            &service.receipts,
            result.idempotency_id,
            identity.user_id,
            "identity_profile",
            "identity",
        ),
    )
    .await
    .map_err(into_connect_error)??;
    Response::ok(RevokeBrowserSessionResponse {
        receipt: receipt.into(),
        ..Default::default()
    })
}

const fn map_error(error: &RevokeBrowserSessionError) -> RpcError {
    match error {
        RevokeBrowserSessionError::Unauthenticated => RpcError::Unauthenticated,
        RevokeBrowserSessionError::IdempotencyConflict => RpcError::FailedPrecondition,
        RevokeBrowserSessionError::Unavailable => RpcError::Unavailable,
    }
}
