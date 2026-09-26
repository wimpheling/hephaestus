//! Personal access token request conversion and error helpers.

use super::super::{
    MediatorAuthenticator, MutationReceipts, RpcError, into_connect_error, mutation_receipt,
    request,
};
use forge_domain::RepositoryId;
use git_capability_domain::GitOperation;
use pat_domain::{
    PersonalAccessTokenId, PersonalAccessTokenMetadata as DomainMetadata,
    PersonalAccessTokenScope as DomainScope,
};
use pat_postgres::PersonalAccessTokenServiceError;
use rpc_proto::messages::hephaestus::{
    common::v1::OpaqueId,
    pat::v1::{
        GitOperation as ProtoGitOperation, PersonalAccessTokenMetadata, PersonalAccessTokenScope,
    },
};
use time::{Duration, OffsetDateTime};

pub(super) fn mutation_identity(
    ctx: &connectrpc::RequestContext,
    authenticator: &MediatorAuthenticator,
    method: &str,
    context: &buffa::MessageField<rpc_proto::messages::hephaestus::common::v1::RequestContext>,
) -> Result<identity_domain::AuthenticatedIdentity, connectrpc::ConnectError> {
    request::mutation_identity(
        ctx,
        authenticator,
        &format!("/hephaestus.pat.v1.PersonalAccessTokenService/{method}"),
        context.as_option(),
    )
    .map_err(into_connect_error)
}

pub(super) async fn identity_receipt(
    receipts: &MutationReceipts,
    identity: &identity_domain::AuthenticatedIdentity,
) -> Result<rpc_proto::messages::hephaestus::common::v1::MutationReceipt, connectrpc::ConnectError>
{
    mutation_receipt(
        receipts,
        identity.request_id,
        identity.user_id,
        "identity_profile",
        "identity",
    )
    .await
}

pub(super) fn scope(
    value: Option<&PersonalAccessTokenScope>,
) -> Result<DomainScope, connectrpc::ConnectError> {
    let value = value.ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?;
    let operations = value
        .operations
        .iter()
        .map(|value| match value.as_known() {
            Some(ProtoGitOperation::Discover) => Ok(GitOperation::Discover),
            Some(ProtoGitOperation::Fetch) => Ok(GitOperation::Fetch),
            Some(ProtoGitOperation::Receive) => Ok(GitOperation::Receive),
            _ => Err(into_connect_error(RpcError::InvalidArgument)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let repositories = (!value.repository_ids.is_empty())
        .then(|| {
            value
                .repository_ids
                .iter()
                .map(|id| {
                    id.value
                        .parse()
                        .map(RepositoryId::from_uuid)
                        .map_err(|_| into_connect_error(RpcError::InvalidArgument))
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    DomainScope::new(operations, repositories)
        .map_err(|_| into_connect_error(RpcError::InvalidArgument))
}

pub(super) fn token_id(
    value: Option<&OpaqueId>,
) -> Result<PersonalAccessTokenId, connectrpc::ConnectError> {
    value
        .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?
        .value
        .parse()
        .map_err(|_| into_connect_error(RpcError::InvalidArgument))
}

pub(super) fn timestamp(
    value: Option<&buffa_types::google::protobuf::Timestamp>,
) -> Result<OffsetDateTime, connectrpc::ConnectError> {
    let value = value.ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?;
    if !(0..1_000_000_000).contains(&value.nanos) {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    let time = OffsetDateTime::from_unix_timestamp(value.seconds)
        .map_err(|_| into_connect_error(RpcError::InvalidArgument))?;
    time.checked_add(Duration::nanoseconds(i64::from(value.nanos)))
        .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))
}

pub(super) fn metadata(value: &DomainMetadata) -> PersonalAccessTokenMetadata {
    PersonalAccessTokenMetadata {
        id: opaque(value.id.to_string()).into(),
        label: value.label.as_str().to_owned(),
        scope: PersonalAccessTokenScope {
            operations: value
                .scope
                .operations()
                .iter()
                .map(|operation| match operation {
                    GitOperation::Discover => ProtoGitOperation::Discover.into(),
                    GitOperation::Fetch => ProtoGitOperation::Fetch.into(),
                    GitOperation::Receive => ProtoGitOperation::Receive.into(),
                })
                .collect(),
            repository_ids: value
                .scope
                .repository_restrictions()
                .into_iter()
                .flatten()
                .map(|id| opaque(id.to_string()))
                .collect(),
            ..Default::default()
        }
        .into(),
        created_at: proto_timestamp(value.created_at).into(),
        expires_at: proto_timestamp(value.expires_at).into(),
        revoked_at: value.revoked_at.map(proto_timestamp).into(),
        last_used_at: value.last_used_at.map(proto_timestamp).into(),
        ..Default::default()
    }
}

fn opaque(value: String) -> OpaqueId {
    OpaqueId {
        value,
        ..Default::default()
    }
}

fn proto_timestamp(value: OffsetDateTime) -> buffa_types::google::protobuf::Timestamp {
    buffa_types::google::protobuf::Timestamp {
        seconds: value.unix_timestamp(),
        nanos: value.nanosecond().cast_signed(),
        ..Default::default()
    }
}

pub(super) fn application_error(
    error: PersonalAccessTokenServiceError,
) -> connectrpc::ConnectError {
    let rpc = match error {
        PersonalAccessTokenServiceError::NotFound => RpcError::NotFound,
        PersonalAccessTokenServiceError::InvalidCredential => RpcError::Unauthenticated,
        PersonalAccessTokenServiceError::InvalidLifecycle => RpcError::FailedPrecondition,
        PersonalAccessTokenServiceError::InvalidRequest => RpcError::InvalidArgument,
        PersonalAccessTokenServiceError::Entropy | PersonalAccessTokenServiceError::Persistence => {
            tracing::error!(%error, "personal access token RPC failed");
            RpcError::Unavailable
        }
    };
    into_connect_error(rpc)
}
