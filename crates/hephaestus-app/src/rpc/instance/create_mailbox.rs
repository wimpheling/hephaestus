use super::InstanceRpc;
use crate::rpc::{RpcError, into_connect_error, mutation_receipt, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use mailbox_postgres::MailboxPersistenceError;
use rpc_proto::messages::hephaestus::instance::v1::{CreateMailboxRequest, CreateMailboxResponse};

const AUDIENCE: &str = "/hephaestus.instance.v1.AgentInstanceService/CreateMailbox";

pub(super) async fn handle(
    service: &InstanceRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, CreateMailboxRequest>,
) -> ServiceResult<CreateMailboxResponse> {
    let request = message.to_owned_message();
    let identity = request::mutation_identity(
        &ctx,
        &service.authenticator,
        AUDIENCE,
        request.context.as_option(),
    )
    .map_err(into_connect_error)?;
    let instance_id = super::parse_id(request.instance_id.as_option())?;
    let mailbox_id = mailbox_postgres::PostgresMailboxRepository::new(service.pool.clone())
        .allocate(&identity, instance_id)
        .await
        .map_err(|error| match error {
            MailboxPersistenceError::IdempotencyConflict => {
                into_connect_error(RpcError::AlreadyExists)
            }
            MailboxPersistenceError::Unavailable => into_connect_error(RpcError::NotFound),
            MailboxPersistenceError::PayloadIntegrity
            | MailboxPersistenceError::UnsupportedContentEncoding
            | MailboxPersistenceError::Provider(_) => into_connect_error(RpcError::Unavailable),
        })?;
    let receipt = mutation_receipt(
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "agent_instance",
        "agent_instance",
    )
    .await?;
    Response::ok(CreateMailboxResponse {
        mailbox_id: super::opaque(mailbox_id.to_string()).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use crate::{
        application::commands::InternalCommandState,
        rpc::{MediatorAuthenticator, MutationReceipts},
    };
    use authz_postgres::PostgresMelangeAuthorizer;
    use buffa::{HasMessageView, Message};
    use bytes::Bytes;
    use connectrpc::{RequestContext as TransportContext, ServiceRequest};
    use http::{Extensions, HeaderMap};
    use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
    use release_domain::{NetworkAccess, RuntimePolicy};
    use release_postgres::ReleaseService;
    use rpc_proto::{
        connect::hephaestus::instance::v1::AgentInstanceService,
        messages::hephaestus::{
            common::v1::{OpaqueId, RequestContext},
            instance::v1::{CreateMailboxRequest, CreateMailboxResponse},
        },
    };
    use secret_postgres::SecretService;
    use secret_store::{EncryptedStore, LocalKeyProvider};
    use serial_test::serial;
    use sqlx::postgres::PgPoolOptions;
    use std::{env, sync::Arc};
    use uuid::Uuid;

    #[tokio::test]
    #[serial]
    async fn create_mailbox_rpc_executes_authorization_idempotency_and_receipt() {
        let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
            // Workspace tests run without services; the dedicated integration
            // harness supplies this URL and therefore executes the proof.
            return;
        };
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(&database_url)
            .await
            .expect("connect real PostgreSQL");
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .expect("apply RPC migration");
        let user_id = UserId::new();
        let organization_id = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        let repository_id = Uuid::new_v4();
        let family_id = Uuid::new_v4();
        let instance_id = runtime_types::AgentInstanceId::new();
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'RPC mailbox owner')")
            .bind(user_id.as_uuid())
            .execute(&pool)
            .await
            .expect("user");
        sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
            .bind(organization_id)
            .bind(format!("rpc-mailbox-{organization_id}"))
            .execute(&pool)
            .await
            .expect("organization");
        sqlx::query(
            "INSERT INTO organization_members (organization_id, user_id, role)
             VALUES ($1, $2, 'owner')",
        )
        .bind(organization_id)
        .bind(user_id.as_uuid())
        .execute(&pool)
        .await
        .expect("organization owner");
        sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
            .bind(project_id)
            .bind(organization_id)
            .bind(format!("rpc-mailbox-{project_id}"))
            .execute(&pool)
            .await
            .expect("project");
        sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
            .bind(project_id)
            .bind(user_id.as_uuid())
            .execute(&pool)
            .await
            .expect("project manager");
        sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
            .bind(repository_id)
            .bind(project_id)
            .bind(format!("rpc-mailbox-{repository_id}"))
            .execute(&pool)
            .await
            .expect("repository");
        sqlx::query(
            "INSERT INTO agent_families (id, repository_id, agent_key)
             VALUES ($1, $2, 'rpc-mailbox')",
        )
        .bind(family_id)
        .bind(repository_id)
        .execute(&pool)
        .await
        .expect("family");
        sqlx::query(
            "INSERT INTO agent_instances (id, project_id, family_id, name, state)
             VALUES ($1, $2, $3, $4, 'active')",
        )
        .bind(instance_id.as_uuid())
        .bind(project_id)
        .bind(family_id)
        .bind(format!("rpc-mailbox-{instance_id}"))
        .execute(&pool)
        .await
        .expect("instance");

        let authorizer = Arc::new(PostgresMelangeAuthorizer);
        let key_provider =
            LocalKeyProvider::new("rpc/v1", [("rpc/v1", [7_u8; 32])]).expect("test key provider");
        let commands = InternalCommandState::new(
            Arc::new(ReleaseService::new(pool.clone(), Arc::clone(&authorizer))),
            Arc::new(SecretService::new(
                pool.clone(),
                EncryptedStore::new(key_provider),
                authorizer,
            )),
            RuntimePolicy {
                vcpus: 1,
                memory_mib: 128,
                network: NetworkAccess::Disabled,
            },
            String::from("rpc-test/v1"),
        );
        let receipts = MutationReceipts::new(
            Arc::new(event_postgres::PostgresMutationReceiptReader::new(
                pool.clone(),
            )),
            [8_u8; 32],
        );
        let service = super::InstanceRpc::new(
            pool.clone(),
            commands,
            MediatorAuthenticator::new(&[9_u8; 32]),
            receipts,
        );
        let identity = AuthenticatedIdentity::new(
            user_id,
            "https://issuer.example",
            format!("rpc-mailbox-{user_id}"),
            serde_json::json!({"email_verified": true}),
            RequestId::new(),
        );
        let first = call(&service, &identity, instance_id, "first")
            .await
            .expect("authorized RPC allocation");
        let mut replay_identity = identity.clone();
        replay_identity.request_id = RequestId::new();
        let replay = call(&service, &replay_identity, instance_id, "first")
            .await
            .expect("idempotent RPC replay");
        assert_eq!(first.mailbox_id, replay.mailbox_id);
        assert!(first.receipt.as_option().is_some());
        assert_eq!(first.receipt, replay.receipt);
        let outsider_id = UserId::new();
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'RPC mailbox outsider')")
            .bind(outsider_id.as_uuid())
            .execute(&pool)
            .await
            .expect("outsider");
        let outsider = AuthenticatedIdentity::new(
            outsider_id,
            "https://issuer.example",
            format!("rpc-mailbox-outsider-{outsider_id}"),
            serde_json::json!({"email_verified": true}),
            RequestId::new(),
        );
        assert!(
            call(&service, &outsider, instance_id, "outsider")
                .await
                .is_err()
        );
    }

    async fn call(
        service: &super::InstanceRpc,
        identity: &AuthenticatedIdentity,
        instance_id: runtime_types::AgentInstanceId,
        idempotency_key: &str,
    ) -> Result<CreateMailboxResponse, connectrpc::ConnectError> {
        let request = CreateMailboxRequest {
            context: RequestContext {
                request_id: OpaqueId {
                    value: identity.request_id.to_string(),
                    ..Default::default()
                }
                .into(),
                idempotency_key: idempotency_key.to_owned(),
                ..Default::default()
            }
            .into(),
            instance_id: OpaqueId {
                value: instance_id.to_string(),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        };
        let body = Bytes::from(request.encode_to_vec());
        let view = CreateMailboxRequest::decode_view(&body).expect("decode RPC request");
        let message = ServiceRequest::from_parts(&view, &body);
        let mut extensions = Extensions::new();
        extensions.insert(identity.clone());
        let context = TransportContext::new(HeaderMap::new()).with_extensions(extensions);
        <super::InstanceRpc as AgentInstanceService>::create_mailbox(service, context, message)
            .await
            .map(|response| response.body)
    }
}
