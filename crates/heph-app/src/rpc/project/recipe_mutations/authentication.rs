use crate::rpc::{MediatorAuthenticator, RpcError, request};
use connectrpc::RequestContext;
use identity_domain::AuthenticatedIdentity;
use rpc_proto::messages::hephaestus::common::v1::RequestContext as WireContext;

pub(super) fn identity(
    ctx: &RequestContext,
    authenticator: &MediatorAuthenticator,
    actual_audience: &str,
    context: Option<&WireContext>,
) -> Result<AuthenticatedIdentity, RpcError> {
    // The middleware authenticates the ACTUAL distinct method audience/session.
    let identity = request::mutation_identity(ctx, authenticator, actual_audience, context)?;
    let session = request::verified_mediator_session(ctx)?;
    if session.user_id != identity.user_id
        || session.parent_session_id.is_none()
        || identity.request_id.as_uuid().is_nil()
    {
        return Err(RpcError::Unauthenticated);
    }
    let context = context.ok_or(RpcError::InvalidArgument)?;
    if !context.__buffa_unknown_fields.is_empty()
        || context
            .request_id
            .as_option()
            .is_some_and(|id| !id.__buffa_unknown_fields.is_empty())
    {
        return Err(RpcError::InvalidArgument);
    }
    // Resume retains the Install logical namespace AFTER actual audience verification.
    let logical = logical_key(identity.user_id, &context.idempotency_key);
    Ok(identity.with_idempotency_id(logical))
}
pub(super) fn logical_key(actor: identity_domain::UserId, key: &str) -> identity_domain::RequestId {
    request::derive_idempotency_id(actor.as_uuid().as_bytes(), super::INSTALL, key)
}
