//! Secret metadata and lifecycle RPC adapters.

use super::{MediatorAuthenticator, MutationReceipts, RpcError};
use crate::application::commands::{InternalCommand, InternalCommandState, dispatch};
use crate::application::secret::SecretApplication;
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use control_plane_postgres::ControlPlanePool as PgPool;
use rpc_proto::{
    connect::hephaestus::secret::v1::SecretService,
    messages::hephaestus::secret::v1::{
        AcceptSecretImportRequest, AcceptSecretImportResponse, CreateSecretRequest,
        CreateSecretResponse, GetProjectSecretAuthorityRequest, GetProjectSecretAuthorityResponse,
        GrantSecretRequest, GrantSecretResponse, ListOrganizationSecretGrantsRequest,
        ListOrganizationSecretGrantsResponse, ListOrganizationSecretsRequest,
        ListOrganizationSecretsResponse, ListProjectSecretsRequest, ListProjectSecretsResponse,
        PurgeSecretRequest, PurgeSecretResponse, RevokeSecretRequest, RevokeSecretResponse,
        RotateSecretRequest, RotateSecretResponse, SetSecretEnabledRequest,
        SetSecretEnabledResponse,
    },
};
use serde_json::Value;

mod budget;
mod model;
mod mutations;
mod queries;

pub struct SecretRpc {
    application: SecretApplication,
    commands: InternalCommandState,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
}

impl SecretRpc {
    pub const fn new(
        pool: PgPool,
        commands: InternalCommandState,
        authenticator: MediatorAuthenticator,
        receipts: MutationReceipts,
    ) -> Self {
        Self {
            application: SecretApplication::new(pool),
            commands,
            authenticator,
            receipts,
        }
    }

    async fn execute(
        &self,
        identity: &identity_domain::AuthenticatedIdentity,
        command: InternalCommand,
    ) -> Result<Value, RpcError> {
        dispatch(&self.commands, identity, command)
            .await
            .map_err(|error| {
                tracing::warn!(
                    actor_id = %identity.user_id,
                    request_id = %identity.request_id,
                    %error,
                    "secret RPC command rejected"
                );
                RpcError::FailedPrecondition
            })
    }
}

#[allow(refining_impl_trait)]
impl SecretService for SecretRpc {
    async fn list_project_secrets(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListProjectSecretsRequest>,
    ) -> ServiceResult<ListProjectSecretsResponse> {
        queries::list_project_secrets(self, ctx, request).await
    }

    async fn list_organization_secrets(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListOrganizationSecretsRequest>,
    ) -> ServiceResult<ListOrganizationSecretsResponse> {
        queries::list_organization_secrets(self, ctx, request).await
    }

    async fn list_organization_secret_grants(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListOrganizationSecretGrantsRequest>,
    ) -> ServiceResult<ListOrganizationSecretGrantsResponse> {
        queries::list_organization_secret_grants(self, ctx, request).await
    }

    async fn get_project_secret_authority(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetProjectSecretAuthorityRequest>,
    ) -> ServiceResult<GetProjectSecretAuthorityResponse> {
        queries::get_project_secret_authority(self, ctx, request).await
    }

    async fn create_secret(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateSecretRequest>,
    ) -> ServiceResult<CreateSecretResponse> {
        mutations::create_secret(self, ctx, request).await
    }

    async fn rotate_secret(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RotateSecretRequest>,
    ) -> ServiceResult<RotateSecretResponse> {
        mutations::rotate_secret(self, ctx, request).await
    }

    async fn revoke_secret(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RevokeSecretRequest>,
    ) -> ServiceResult<RevokeSecretResponse> {
        mutations::revoke_secret(self, ctx, request).await
    }

    async fn set_secret_enabled(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetSecretEnabledRequest>,
    ) -> ServiceResult<SetSecretEnabledResponse> {
        mutations::set_secret_enabled(self, ctx, request).await
    }

    async fn purge_secret(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PurgeSecretRequest>,
    ) -> ServiceResult<PurgeSecretResponse> {
        mutations::purge_secret(self, ctx, request).await
    }

    async fn grant_secret(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GrantSecretRequest>,
    ) -> ServiceResult<GrantSecretResponse> {
        mutations::grant_secret(self, ctx, request).await
    }

    async fn accept_secret_import(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AcceptSecretImportRequest>,
    ) -> ServiceResult<AcceptSecretImportResponse> {
        mutations::accept_secret_import(self, ctx, request).await
    }
}
