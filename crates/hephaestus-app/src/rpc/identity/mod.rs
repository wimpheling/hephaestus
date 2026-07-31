//! Identity-resolution Connect adapter.

use super::{
    BootstrapIdentity, MediatorAuthenticator, MutationReceipts, RpcError, into_connect_error,
    mutation_receipt,
};
use crate::application::identity::{IdentityApplication, ResolveIdentity, ResolveIdentityError};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use identity_application::IdempotentIdentityResolver;
use identity_domain::RequestId;
use rpc_proto::{
    connect::hephaestus::identity::v1::IdentityService,
    messages::hephaestus::{
        common::v1::OpaqueId,
        identity::v1::{ResolveIdentityRequest, ResolveIdentityResponse},
    },
};
use std::{future::Future, str::FromStr, sync::Arc};

const AUDIENCE: &str = "/hephaestus.identity.v1.IdentityService/ResolveIdentity";
const MAX_ISSUER_BYTES: usize = 2_048;
const MAX_SUBJECT_BYTES: usize = 2_048;
const MAX_DISPLAY_NAME_BYTES: usize = 200;
const MAX_EMAIL_BYTES: usize = 320;
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 256;

/// Generated identity service implementation.
pub struct IdentityRpc {
    application: IdentityApplication,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
}

impl IdentityRpc {
    /// Creates the identity adapter and its application operation.
    pub fn new(
        resolver: Arc<dyn IdempotentIdentityResolver>,
        authenticator: MediatorAuthenticator,
        receipts: MutationReceipts,
    ) -> Self {
        Self {
            application: IdentityApplication::new(resolver),
            authenticator,
            receipts,
        }
    }
}

impl IdentityService for IdentityRpc {
    fn resolve_identity<'a>(
        &'a self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ResolveIdentityRequest>,
    ) -> impl Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<ResolveIdentityResponse> + Send + use<'a>,
        >,
    > + Send {
        let request = request.to_owned_message();
        async move {
            validate_identity_fields(&request).map_err(into_connect_error)?;
            self.authenticator
                .authenticate_bootstrap(
                    ctx.headers(),
                    AUDIENCE,
                    &BootstrapIdentity {
                        issuer: &request.issuer,
                        subject: &request.subject,
                        display_name: &request.display_name,
                        email: &request.email,
                        email_verified: request.email_verified,
                    },
                )
                .map_err(|_| into_connect_error(RpcError::Unauthenticated))?;
            let request_id = parse_request_context(&request).map_err(into_connect_error)?;
            let idempotency_seed = identity_domain::mutation_idempotency_seed(
                AUDIENCE,
                &request
                    .context
                    .as_option()
                    .expect("validated request context")
                    .idempotency_key,
            );
            let resolved = self
                .application
                .resolve_identity(ResolveIdentity {
                    request_id,
                    idempotency_seed,
                    issuer: request.issuer,
                    subject: request.subject,
                    display_name: request.display_name,
                    email: request.email,
                    email_verified: request.email_verified,
                })
                .await
                .map_err(map_application_error)
                .map_err(into_connect_error)?;
            let receipt = mutation_receipt(
                &self.receipts,
                resolved.idempotency_id,
                resolved.user_id,
                "identity_profile",
                "identity",
            )
            .await?;
            Response::ok(ResolveIdentityResponse {
                user_id: OpaqueId {
                    value: resolved.user_id.to_string(),
                    ..Default::default()
                }
                .into(),
                display_name: resolved.display_name,
                receipt: receipt.into(),
                ..Default::default()
            })
        }
    }
}

fn validate_identity_fields(request: &ResolveIdentityRequest) -> Result<(), RpcError> {
    if !bounded(&request.issuer, MAX_ISSUER_BYTES)
        || !bounded(&request.subject, MAX_SUBJECT_BYTES)
        || !bounded(&request.display_name, MAX_DISPLAY_NAME_BYTES)
        || request.email.len() > MAX_EMAIL_BYTES
    {
        return Err(RpcError::InvalidArgument);
    }
    Ok(())
}

fn parse_request_context(request: &ResolveIdentityRequest) -> Result<RequestId, RpcError> {
    let context = request
        .context
        .as_option()
        .ok_or(RpcError::InvalidArgument)?;
    let request_id = context
        .request_id
        .as_option()
        .ok_or(RpcError::InvalidArgument)?;
    if !bounded(&context.idempotency_key, MAX_IDEMPOTENCY_KEY_BYTES) {
        return Err(RpcError::InvalidArgument);
    }
    RequestId::from_str(&request_id.value).map_err(|_| RpcError::InvalidArgument)
}

const fn bounded(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum
}

fn map_application_error(error: ResolveIdentityError) -> RpcError {
    match error {
        ResolveIdentityError::PermissionDenied => RpcError::PermissionDenied,
        ResolveIdentityError::IdempotencyConflict => RpcError::FailedPrecondition,
        ResolveIdentityError::Provider(source) => {
            tracing::error!(error = %source, "identity resolution persistence failed");
            RpcError::Unavailable
        }
    }
}
