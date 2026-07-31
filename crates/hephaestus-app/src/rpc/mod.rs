//! Connect RPC transport authentication and service composition.

mod artifact;
mod auth;
mod build;
mod error;
// The event adapter is shared with the single outbound product-event adapter.
#[allow(clippy::redundant_pub_crate)]
pub(crate) mod event;
mod identity;
mod instance;
mod organization;
mod project;
mod release;
mod repository;
mod repository_browser;
mod request;
mod run;
mod secret;

pub use auth::{
    BootstrapIdentity, MediatorAssertionError, MediatorAuthenticator, MediatorPrincipal,
};
pub use error::{RpcError, into_connect_error};

use connectrpc::{ConnectRpcService, DeadlinePolicy, Limits, Router};
use connectrpc_reflection::Reflector;
use control_plane_postgres::ControlPlanePool as PgPool;
use event_application::MutationReceiptReader;
use forge_service::GitStorage;
use identity_application::IdempotentIdentityResolver;
use release_artifact_store::LocalArtifactStore;
use rpc_proto::connect::hephaestus::identity::v1::IdentityServiceExt;
use std::{path::PathBuf, sync::Arc, time::Duration};

use sha2::{Digest, Sha256};

const MEDIATOR_KEY_DOMAIN: &[u8] = b"hephaestus-rpc-mediator-v1\0";
const GLOBAL_MAX_REQUEST_BYTES: usize = 1024 * 1024;
const GLOBAL_MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_DEADLINE_SECONDS: u64 = 60;
const DEFAULT_DEADLINE_SECONDS: u64 = 30;
const STREAM_IDLE_SECONDS: u64 = 10;

#[derive(Clone)]
struct MutationReceipts {
    application: Arc<dyn MutationReceiptReader>,
    cursor_codec: crate::event_cursor::EventCursorCodec,
}

impl MutationReceipts {
    fn new(application: Arc<dyn MutationReceiptReader>, cursor_key: [u8; 32]) -> Self {
        Self {
            application,
            cursor_codec: crate::event_cursor::EventCursorCodec::new(cursor_key),
        }
    }

    async fn load(
        &self,
        occurrence_id: identity_domain::RequestId,
        actor_id: identity_domain::UserId,
        aggregate_type: &str,
        primary_scope_kind: &str,
    ) -> Result<rpc_proto::messages::hephaestus::common::v1::MutationReceipt, ConnectReceiptError>
    {
        let row = self
            .application
            .load(occurrence_id, actor_id, aggregate_type, primary_scope_kind)
            .await
            .map_err(ConnectReceiptError::Application)?;
        Ok(
            rpc_proto::messages::hephaestus::common::v1::MutationReceipt {
                event_id: rpc_proto::messages::hephaestus::common::v1::OpaqueId {
                    value: row.event_id.to_string(),
                    ..Default::default()
                }
                .into(),
                committed_cursor: rpc_proto::messages::hephaestus::common::v1::Cursor {
                    value: self
                        .cursor_codec
                        .encode(&row.scope_kind, row.scope_id, row.cursor),
                    ..Default::default()
                }
                .into(),
                aggregate_version: u64::try_from(row.aggregate_version)
                    .map_err(|_| ConnectReceiptError::InvalidVersion)?,
                ..Default::default()
            },
        )
    }
}

#[derive(Debug, thiserror::Error)]
enum ConnectReceiptError {
    #[error("mutation receipt application operation failed")]
    Application(#[source] event_application::MutationReceiptError),
    #[error("committed mutation event version is invalid")]
    InvalidVersion,
}

async fn mutation_receipt(
    receipts: &MutationReceipts,
    occurrence_id: identity_domain::RequestId,
    actor_id: identity_domain::UserId,
    aggregate_type: &str,
    primary_scope_kind: &str,
) -> Result<rpc_proto::messages::hephaestus::common::v1::MutationReceipt, connectrpc::ConnectError>
{
    receipts
        .load(
            occurrence_id,
            actor_id,
            aggregate_type,
            primary_scope_kind,
        )
        .await
        .map_err(|error| {
            tracing::error!(%occurrence_id, %aggregate_type, %primary_scope_kind, %error, "mutation receipt unavailable");
            into_connect_error(RpcError::Internal)
        })
}

/// Derives the domain-separated HS256 key shared with the Phoenix mediator.
#[must_use]
pub fn mediator_signing_key(internal_token: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(MEDIATOR_KEY_DOMAIN);
    digest.update(internal_token);
    digest.finalize().into()
}

/// Application dependencies shared by the generated Connect services.
pub(crate) struct ApplicationDependencies {
    pool: PgPool,
    mutation_receipt_reader: Arc<dyn MutationReceiptReader>,
    identity_resolver: Arc<dyn IdempotentIdentityResolver>,
}

impl ApplicationDependencies {
    pub(crate) fn new(
        pool: PgPool,
        mutation_receipt_reader: Arc<dyn MutationReceiptReader>,
        identity_resolver: Arc<dyn IdempotentIdentityResolver>,
    ) -> Self {
        Self {
            pool,
            mutation_receipt_reader,
            identity_resolver,
        }
    }
}

/// Builds the generated Connect services and reflection endpoints.
// The generated service inventory is clearest as one auditable router composition.
#[allow(clippy::too_many_lines)]
pub(crate) fn service(
    applications: ApplicationDependencies,
    storage: Arc<GitStorage>,
    artifact_store: LocalArtifactStore,
    result_artifact_root: PathBuf,
    mediator_signing_key: &[u8],
    commands: crate::application::commands::InternalCommandState,
    event_wakeups: Arc<dyn crate::application::event::EventWakeupSource>,
) -> Result<ConnectRpcService, RpcInitializationError> {
    let cursor_key: [u8; 32] = mediator_signing_key.try_into().map_err(|_| {
        RpcInitializationError::Descriptor(String::from("invalid event cursor key"))
    })?;
    let mutation_receipts = MutationReceipts::new(applications.mutation_receipt_reader, cursor_key);
    let pool = &applications.pool;
    let identity = Arc::new(identity::IdentityRpc::new(
        Arc::clone(&applications.identity_resolver),
        MediatorAuthenticator::new(mediator_signing_key),
        mutation_receipts.clone(),
    ));
    let organization = Arc::new(organization::OrganizationRpc::new(
        pool.clone(),
        MediatorAuthenticator::new(mediator_signing_key),
    ));
    let instance = Arc::new(instance::InstanceRpc::new(
        pool.clone(),
        commands.clone(),
        MediatorAuthenticator::new(mediator_signing_key),
        mutation_receipts.clone(),
    ));
    let secret = Arc::new(secret::SecretRpc::new(
        pool.clone(),
        commands,
        MediatorAuthenticator::new(mediator_signing_key),
        mutation_receipts.clone(),
    ));
    let router = IdentityServiceExt::register(identity, Router::new());
    let router = rpc_proto::connect::hephaestus::instance::v1::AgentInstanceServiceExt::register(
        instance, router,
    );
    let router = rpc_proto::connect::hephaestus::organization::v1::OrganizationServiceExt::register(
        organization,
        router,
    );
    let router =
        rpc_proto::connect::hephaestus::secret::v1::SecretServiceExt::register(secret, router);
    let router = project::register(
        router,
        pool.clone(),
        MediatorAuthenticator::new(mediator_signing_key),
    );
    let router = repository::register(
        router,
        pool.clone(),
        MediatorAuthenticator::new(mediator_signing_key),
    );
    let router = repository_browser::register(
        router,
        pool.clone(),
        storage,
        MediatorAuthenticator::new(mediator_signing_key),
    );
    let router = build::register(
        router,
        pool.clone(),
        MediatorAuthenticator::new(mediator_signing_key),
        mutation_receipts.clone(),
    );
    let router = release::register(
        router,
        pool.clone(),
        MediatorAuthenticator::new(mediator_signing_key),
    );
    let router = artifact::register(
        router,
        pool.clone(),
        artifact_store,
        mediator_signing_key,
        MediatorAuthenticator::new(mediator_signing_key),
    );
    let router = run::register(
        router,
        pool.clone(),
        result_artifact_root,
        MediatorAuthenticator::new(mediator_signing_key),
        mutation_receipts,
    );
    let router = event::register(
        router,
        pool.clone(),
        MediatorAuthenticator::new(mediator_signing_key),
        event_wakeups,
        cursor_key,
    );
    let descriptor_pool = rpc_proto::descriptor_pool()
        .map_err(|error| RpcInitializationError::Descriptor(error.to_string()))?;
    let reflector = Reflector::from_descriptor_pool(Arc::new(descriptor_pool))?;
    let router = connectrpc_reflection::install(router, reflector);
    Ok(ConnectRpcService::new(router)
        .with_limits(
            Limits::default()
                .max_request_body_size(GLOBAL_MAX_REQUEST_BYTES)
                .max_message_size(GLOBAL_MAX_MESSAGE_BYTES),
        )
        .with_deadline_policy(
            DeadlinePolicy::new()
                .with_max(Duration::from_secs(MAX_DEADLINE_SECONDS))
                .with_default_timeout(Duration::from_secs(DEFAULT_DEADLINE_SECONDS))
                .with_enforce_on_streams(true)
                .with_inter_message_timeout(Duration::from_secs(STREAM_IDLE_SECONDS)),
        ))
}

/// Failure to construct the static RPC service graph.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RpcInitializationError {
    /// The checked-in descriptor set could not be decoded.
    #[error("RPC descriptor set is invalid: {0}")]
    Descriptor(String),
    /// The reflection index could not be built.
    #[error("RPC reflection configuration is invalid")]
    Reflection(#[from] connectrpc_reflection::ReflectionError),
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_DEADLINE_SECONDS, GLOBAL_MAX_MESSAGE_BYTES, GLOBAL_MAX_REQUEST_BYTES,
        MAX_DEADLINE_SECONDS, STREAM_IDLE_SECONDS,
    };
    use connectrpc_reflection::{
        Reflector, SERVER_REFLECTION_SERVICE_NAME, SERVER_REFLECTION_V1ALPHA_SERVICE_NAME,
    };
    use std::sync::Arc;

    #[test]
    fn checked_in_descriptors_expose_application_and_reflection_services() {
        let reflector = Reflector::from_descriptor_pool(Arc::new(
            rpc_proto::descriptor_pool().expect("checked-in descriptor pool"),
        ))
        .expect("reflection index");
        let services = reflector.service_names();
        assert!(
            services
                .iter()
                .any(|service| service == "hephaestus.identity.v1.IdentityService")
        );
        assert!(
            services
                .iter()
                .any(|service| service == SERVER_REFLECTION_SERVICE_NAME)
        );
        assert!(
            services
                .iter()
                .any(|service| service == SERVER_REFLECTION_V1ALPHA_SERVICE_NAME)
        );
    }

    #[test]
    fn transport_limits_leave_preview_envelope_headroom_and_bound_deadlines() {
        const MAX_ARTIFACT_PREVIEW_BYTES: usize = 1024 * 1024;
        const MAX_INSTANCE_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
        assert_eq!(GLOBAL_MAX_REQUEST_BYTES, MAX_ARTIFACT_PREVIEW_BYTES);
        assert_eq!(GLOBAL_MAX_MESSAGE_BYTES, MAX_INSTANCE_RESPONSE_BYTES);
        assert_eq!(MAX_DEADLINE_SECONDS, 60);
        assert_eq!(DEFAULT_DEADLINE_SECONDS, 30);
        assert_eq!(STREAM_IDLE_SECONDS, 10);
    }
}
