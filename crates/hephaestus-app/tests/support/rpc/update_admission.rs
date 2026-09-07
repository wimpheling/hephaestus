//! Generated-client update admission request helpers.
//!
//! This module keeps JWT construction and Connect RPC request shapes at the
//! transport boundary; SQL and durable lifecycle assertions live in the
//! sibling `support::update_admission` module.

use connectrpc::{
    Protocol,
    client::{CallOptions, ClientConfig, Http2Connection, SharedHttp2Connection},
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rpc_proto::{
    connect::hephaestus::instance::v1::AgentInstanceServiceClient,
    messages::hephaestus::{
        common::v1::{NetworkPolicy, OpaqueId, RequestContext, RuntimePolicy},
        instance::v1::{
            CreateUpdateRequest, RecoverUpdateRequest, RecoveryAction as ProtoRecoveryAction,
        },
    },
};
use time::OffsetDateTime;
use uuid::Uuid;

/// Durable identities needed to exercise the production update command.
#[derive(Clone, Copy, Debug)]
#[allow(clippy::struct_field_names)] // UUID suffixes make fixture identity roles explicit.
pub struct UpdateAdmissionInstance {
    /// Instance whose update lifecycle is under test.
    pub instance_id: Uuid,
    /// Revision used by the active normal run and as the update expectation.
    pub revision_id: Uuid,
    /// Published release attached to the active normal run.
    pub release_id: Uuid,
    /// Release agent used by the candidate update.
    pub release_agent_id: Uuid,
    /// Attachment owned by the active normal run.
    pub attachment_id: Uuid,
}

/// Durable identities observed while the admission/recovery sequence runs.
#[derive(Clone, Copy, Debug)]
#[allow(clippy::struct_field_names)] // UUID suffixes make observed lifecycle roles explicit.
pub struct UpdateAdmissionResult {
    /// Update accepted while the normal run was still active.
    pub update_id: Uuid,
    /// First hook run admitted after normal-run cleanup.
    pub initial_hook_run_id: Uuid,
    /// Fresh hook run admitted by the first explicit retry.
    pub retried_hook_run_id: Uuid,
    /// Fresh hook run admitted by the authorized-owner retry.
    pub owner_recovery_hook_run_id: Uuid,
}

/// Opaque generated-client handle passed to the durable admission driver.
pub struct UpdateRpcClient(AgentInstanceServiceClient<SharedHttp2Connection>);

/// Recovery action understood by the admission driver.
#[derive(Clone, Copy, Debug)]
pub enum RecoveryAction {
    /// Retry a compatibility-unknown update.
    Retry,
}

pub async fn app_instance_client(running: &hephaestus_app::RunningHephaestus) -> UpdateRpcClient {
    let uri: axum::http::Uri = format!("http://{}", running.http_addr())
        .parse()
        .expect("app RPC URI");
    let connection = Http2Connection::connect_plaintext(uri.clone())
        .await
        .expect("app RPC connection")
        .shared(4);
    UpdateRpcClient(AgentInstanceServiceClient::new(
        connection,
        ClientConfig::new(uri).with_protocol(Protocol::Connect),
    ))
}

pub async fn create_draining_update(
    client: &UpdateRpcClient,
    instance: &UpdateAdmissionInstance,
    actor: uuid::Uuid,
) -> (uuid::Uuid, Option<uuid::Uuid>) {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let token = encode(
        &Header::new(Algorithm::HS256),
        &serde_json::json!({
            "iss": "hephaestus-web-mediator",
            "sub": actor.to_string(),
            "aud": "/hephaestus.instance.v1.AgentInstanceService/CreateUpdate",
            "iat": now,
            "nbf": now,
            "exp": now + 25,
            "jti": uuid::Uuid::new_v4().to_string()
        }),
        &EncodingKey::from_secret(&hephaestus_app::rpc::mediator_signing_key(
            b"golden-internal-command-token-with-sufficient-entropy",
        )),
    )
    .expect("sign app update command token");
    let update = client
        .0
        .create_update_with_options(
            CreateUpdateRequest {
                context: RequestContext {
                    request_id: OpaqueId {
                        value: uuid::Uuid::new_v4().to_string(),
                        ..Default::default()
                    }
                    .into(),
                    idempotency_key: String::from("app-update-admission-regression"),
                    ..Default::default()
                }
                .into(),
                instance_id: OpaqueId {
                    value: instance.instance_id.to_string(),
                    ..Default::default()
                }
                .into(),
                expected_revision_id: OpaqueId {
                    value: instance.revision_id.to_string(),
                    ..Default::default()
                }
                .into(),
                candidate_release_agent_id: OpaqueId {
                    value: instance.release_agent_id.to_string(),
                    ..Default::default()
                }
                .into(),
                selected_policy: RuntimePolicy {
                    vcpus: 1,
                    memory_mib: 512,
                    network: NetworkPolicy::Disabled.into(),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            },
            CallOptions::default().with_header("authorization", format!("Bearer {token}")),
        )
        .await
        .expect("accepted draining app update")
        .into_owned();
    (
        uuid::Uuid::parse_str(&update.update_id.as_option().expect("update ID").value)
            .expect("update ID UUID"),
        update
            .hook_run_id
            .as_option()
            .map(|id| uuid::Uuid::parse_str(&id.value).expect("hook run ID UUID")),
    )
}

/// Sends one generated-client recovery request for the supplied actor.
pub async fn recover_update(
    client: &UpdateRpcClient,
    actor: Uuid,
    update_id: Uuid,
    action: RecoveryAction,
    idempotency_key: &str,
) {
    let action = match action {
        RecoveryAction::Retry => ProtoRecoveryAction::Retry,
    };
    let audience = "/hephaestus.instance.v1.AgentInstanceService/RecoverUpdate";
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let token = encode(
        &Header::new(Algorithm::HS256),
        &serde_json::json!({
            "iss": "hephaestus-web-mediator",
            "sub": actor.to_string(),
            "aud": audience,
            "iat": now,
            "nbf": now,
            "exp": now + 25,
            "jti": uuid::Uuid::new_v4().to_string()
        }),
        &EncodingKey::from_secret(&hephaestus_app::rpc::mediator_signing_key(
            b"golden-internal-command-token-with-sufficient-entropy",
        )),
    )
    .expect("sign app update recovery token");
    client
        .0
        .recover_update_with_options(
            RecoverUpdateRequest {
                context: RequestContext {
                    request_id: OpaqueId {
                        value: uuid::Uuid::new_v4().to_string(),
                        ..Default::default()
                    }
                    .into(),
                    idempotency_key: idempotency_key.to_owned(),
                    ..Default::default()
                }
                .into(),
                update_id: OpaqueId {
                    value: update_id.to_string(),
                    ..Default::default()
                }
                .into(),
                action: action.into(),
                ..Default::default()
            },
            CallOptions::default().with_header("authorization", format!("Bearer {token}")),
        )
        .await
        .expect("authorized update recovery");
}
