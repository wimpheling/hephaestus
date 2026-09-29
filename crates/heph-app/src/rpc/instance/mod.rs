//! Agent-instance RPC adapters.

mod attachment;
mod budget;
mod create_mailbox;
mod get_instance;
mod helpers;
mod lifecycle;
mod mailbox;
mod secrets;

use super::{MediatorAuthenticator, MutationReceipts, RpcError};
use crate::application::commands::{InternalCommand, InternalCommandState, dispatch};
use crate::application::instance::InstanceApplication;
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use control_plane_postgres::ControlPlanePool as PgPool;
use rpc_proto::{
    connect::hephaestus::instance::v1::AgentInstanceService,
    messages::hephaestus::instance::v1::{
        BindSecretRequest, BindSecretResponse, ControlMailboxRequest, ControlMailboxResponse,
        CreateAttachmentRequest, CreateAttachmentResponse, CreateMailboxRequest,
        CreateMailboxResponse, CreateUpdateRequest, CreateUpdateResponse,
        DeclareBrokeredHttpsRuleRequest, DeclareBrokeredHttpsRuleResponse, GetInstanceRequest,
        GetInstanceResponse, ImportAgentRequest, ImportAgentResponse, RecoverUpdateRequest,
        RecoverUpdateResponse, RemoveAttachmentRequest, RemoveAttachmentResponse,
        ReviseCapabilitiesRequest, ReviseCapabilitiesResponse, ReviseInstanceRequest,
        ReviseInstanceResponse, SetAttachmentEnabledRequest, SetAttachmentEnabledResponse,
    },
};
use serde_json::Value;

/// Generated instance service backed by the existing release application.
pub struct InstanceRpc {
    application: InstanceApplication,
    pool: PgPool,
    commands: InternalCommandState,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
}

impl InstanceRpc {
    /// Creates an instance service using the shared application command state.
    pub fn new(
        pool: PgPool,
        commands: InternalCommandState,
        authenticator: MediatorAuthenticator,
        receipts: MutationReceipts,
    ) -> Self {
        Self {
            application: InstanceApplication::new(pool.clone()),
            pool,
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
                    "RPC command rejected"
                );
                RpcError::FailedPrecondition
            })
    }
}

// Generated traits hide an Encodable response; concrete message bodies keep
// each adapter readable while refining only this implementation's opaque type.

#[allow(refining_impl_trait)]
impl AgentInstanceService for InstanceRpc {
    async fn get_instance(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetInstanceRequest>,
    ) -> ServiceResult<GetInstanceResponse> {
        get_instance::handle(self, ctx, request).await
    }

    async fn create_mailbox(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateMailboxRequest>,
    ) -> ServiceResult<CreateMailboxResponse> {
        create_mailbox::handle(self, ctx, request).await
    }

    async fn control_mailbox(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ControlMailboxRequest>,
    ) -> ServiceResult<ControlMailboxResponse> {
        mailbox::control_mailbox(self, ctx, request).await
    }

    async fn import_agent(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ImportAgentRequest>,
    ) -> ServiceResult<ImportAgentResponse> {
        lifecycle::import_agent(self, ctx, request).await
    }

    async fn create_attachment(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateAttachmentRequest>,
    ) -> ServiceResult<CreateAttachmentResponse> {
        attachment::create_attachment(self, ctx, request).await
    }

    async fn set_attachment_enabled(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetAttachmentEnabledRequest>,
    ) -> ServiceResult<SetAttachmentEnabledResponse> {
        attachment::set_attachment_enabled(self, ctx, request).await
    }

    async fn remove_attachment(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveAttachmentRequest>,
    ) -> ServiceResult<RemoveAttachmentResponse> {
        attachment::remove_attachment(self, ctx, request).await
    }

    async fn revise_instance(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReviseInstanceRequest>,
    ) -> ServiceResult<ReviseInstanceResponse> {
        lifecycle::revise_instance(self, ctx, request).await
    }

    async fn create_update(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateUpdateRequest>,
    ) -> ServiceResult<CreateUpdateResponse> {
        lifecycle::create_update(self, ctx, request).await
    }

    async fn recover_update(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RecoverUpdateRequest>,
    ) -> ServiceResult<RecoverUpdateResponse> {
        lifecycle::recover_update(self, ctx, request).await
    }

    async fn bind_secret(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BindSecretRequest>,
    ) -> ServiceResult<BindSecretResponse> {
        secrets::bind_secret(self, ctx, request).await
    }

    async fn declare_brokered_https_rule(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeclareBrokeredHttpsRuleRequest>,
    ) -> ServiceResult<DeclareBrokeredHttpsRuleResponse> {
        secrets::declare_brokered_https_rule(self, ctx, request).await
    }

    async fn revise_capabilities(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReviseCapabilitiesRequest>,
    ) -> ServiceResult<ReviseCapabilitiesResponse> {
        secrets::revise_capabilities(self, ctx, request).await
    }
}
