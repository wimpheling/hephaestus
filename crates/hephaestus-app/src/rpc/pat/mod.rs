//! Developer personal access token RPC boundary.

use super::{
    MediatorAuthenticator, MutationReceipts, RpcError, into_connect_error, mutation_receipt,
    request,
};
use connectrpc::{RequestContext, Response, Router, ServiceRequest, ServiceResult};
use control_plane_postgres::ControlPlanePool as PgPool;
use forge_domain::RepositoryId;
use git_capability_domain::GitOperation;
use pat_domain::{
    PersonalAccessTokenId, PersonalAccessTokenLabel, PersonalAccessTokenMetadata as DomainMetadata,
    PersonalAccessTokenScope as DomainScope,
};
use pat_postgres::{
    CreatePersonalAccessToken as CreateCommand, PersonalAccessTokenServiceError,
    PostgresPersonalAccessTokenService, RotatePersonalAccessToken as RotateCommand,
};
use rpc_proto::{
    connect::hephaestus::pat::v1::{PersonalAccessTokenService, PersonalAccessTokenServiceExt},
    messages::hephaestus::{
        common::v1::{OpaqueId, PageResponse},
        pat::v1::{
            CreatePersonalAccessTokenRequest, CreatePersonalAccessTokenResponse,
            GitOperation as ProtoGitOperation, ListPersonalAccessTokensRequest,
            ListPersonalAccessTokensResponse, PersonalAccessTokenMetadata,
            PersonalAccessTokenScope, PersonalAccessTokenValue, RevokePersonalAccessTokenRequest,
            RevokePersonalAccessTokenResponse, RotatePersonalAccessTokenRequest,
            RotatePersonalAccessTokenResponse,
        },
    },
};
use std::str::FromStr;
use time::{Duration, OffsetDateTime};

const DEFAULT_PAGE_SIZE: usize = 50;
const MAX_PAGE_SIZE: usize = 100;

pub struct PersonalAccessTokenRpc {
    application: PostgresPersonalAccessTokenService,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
}

impl PersonalAccessTokenRpc {
    const fn new(
        pool: PgPool,
        authenticator: MediatorAuthenticator,
        receipts: MutationReceipts,
    ) -> Self {
        Self {
            application: PostgresPersonalAccessTokenService::new(pool),
            authenticator,
            receipts,
        }
    }
}

pub fn register(
    router: Router,
    pool: PgPool,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
) -> Router {
    PersonalAccessTokenServiceExt::register(
        std::sync::Arc::new(PersonalAccessTokenRpc::new(pool, authenticator, receipts)),
        router,
    )
}

#[allow(refining_impl_trait)]
impl PersonalAccessTokenService for PersonalAccessTokenRpc {
    async fn list_personal_access_tokens(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListPersonalAccessTokensRequest>,
    ) -> ServiceResult<ListPersonalAccessTokensResponse> {
        let identity = request::query_identity(
            &ctx,
            &self.authenticator,
            "/hephaestus.pat.v1.PersonalAccessTokenService/ListPersonalAccessTokens",
        )
        .map_err(into_connect_error)?;
        let request = request.to_owned_message();
        let requested_size = request.page.as_option().map_or(DEFAULT_PAGE_SIZE, |page| {
            usize::try_from(page.page_size).unwrap_or(usize::MAX)
        });
        let page_size = if requested_size == 0 {
            DEFAULT_PAGE_SIZE
        } else {
            requested_size
        };
        if page_size > MAX_PAGE_SIZE {
            return Err(into_connect_error(RpcError::InvalidArgument));
        }
        let after = request
            .page
            .as_option()
            .filter(|page| !page.page_token.is_empty())
            .map(|page| PersonalAccessTokenId::from_str(&page.page_token))
            .transpose()
            .map_err(|_| into_connect_error(RpcError::InvalidArgument))?;
        let values = self
            .application
            .list(&identity)
            .await
            .map_err(application_error)?;
        let start = match after {
            Some(after) => values
                .iter()
                .position(|value| value.id == after)
                .and_then(|index| index.checked_add(1))
                .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?,
            None => 0,
        };
        let end = start.saturating_add(page_size).min(values.len());
        let next_page_token = (end < values.len()).then(|| values[end - 1].id.to_string());
        Response::ok(ListPersonalAccessTokensResponse {
            tokens: values[start..end].iter().map(metadata).collect(),
            page: PageResponse {
                next_page_token: next_page_token.unwrap_or_default(),
                stable_order: String::from("created_at desc,id"),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
    }

    async fn create_personal_access_token(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreatePersonalAccessTokenRequest>,
    ) -> ServiceResult<CreatePersonalAccessTokenResponse> {
        let request = request.to_owned_message();
        let identity = mutation_identity(
            &ctx,
            &self.authenticator,
            "CreatePersonalAccessToken",
            &request.context,
        )?;
        let issued = self
            .application
            .create(
                &identity,
                CreateCommand {
                    label: PersonalAccessTokenLabel::parse(request.label)
                        .map_err(|_| into_connect_error(RpcError::InvalidArgument))?,
                    scope: scope(request.scope.as_option())?,
                    expires_at: timestamp(request.expires_at.as_option())?,
                },
            )
            .await
            .map_err(application_error)?;
        let receipt = identity_receipt(&self.receipts, &identity).await?;
        Response::ok(CreatePersonalAccessTokenResponse {
            token: metadata(&issued.metadata).into(),
            value: PersonalAccessTokenValue {
                value: issued.token.expose().into_bytes(),
                ..Default::default()
            }
            .into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }

    async fn rotate_personal_access_token(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RotatePersonalAccessTokenRequest>,
    ) -> ServiceResult<RotatePersonalAccessTokenResponse> {
        let request = request.to_owned_message();
        let identity = mutation_identity(
            &ctx,
            &self.authenticator,
            "RotatePersonalAccessToken",
            &request.context,
        )?;
        let issued = self
            .application
            .rotate(
                &identity,
                RotateCommand {
                    token_id: token_id(request.token_id.as_option())?,
                    label: PersonalAccessTokenLabel::parse(request.label)
                        .map_err(|_| into_connect_error(RpcError::InvalidArgument))?,
                    scope: scope(request.scope.as_option())?,
                    expires_at: timestamp(request.expires_at.as_option())?,
                },
            )
            .await
            .map_err(application_error)?;
        let receipt = identity_receipt(&self.receipts, &identity).await?;
        Response::ok(RotatePersonalAccessTokenResponse {
            token: metadata(&issued.metadata).into(),
            value: PersonalAccessTokenValue {
                value: issued.token.expose().into_bytes(),
                ..Default::default()
            }
            .into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }

    async fn revoke_personal_access_token(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RevokePersonalAccessTokenRequest>,
    ) -> ServiceResult<RevokePersonalAccessTokenResponse> {
        let request = request.to_owned_message();
        let identity = mutation_identity(
            &ctx,
            &self.authenticator,
            "RevokePersonalAccessToken",
            &request.context,
        )?;
        let revoked = self
            .application
            .revoke(&identity, token_id(request.token_id.as_option())?)
            .await
            .map_err(application_error)?;
        let receipt = identity_receipt(&self.receipts, &identity).await?;
        Response::ok(RevokePersonalAccessTokenResponse {
            token: metadata(&revoked).into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }
}

fn mutation_identity(
    ctx: &RequestContext,
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

async fn identity_receipt(
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

fn scope(
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

fn token_id(value: Option<&OpaqueId>) -> Result<PersonalAccessTokenId, connectrpc::ConnectError> {
    value
        .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?
        .value
        .parse()
        .map_err(|_| into_connect_error(RpcError::InvalidArgument))
}

fn timestamp(
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

fn metadata(value: &DomainMetadata) -> PersonalAccessTokenMetadata {
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

fn application_error(error: PersonalAccessTokenServiceError) -> connectrpc::ConnectError {
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

#[cfg(test)]
mod tests {
    use super::{scope, timestamp};
    use rpc_proto::messages::hephaestus::pat::v1::{GitOperation, PersonalAccessTokenScope};

    #[test]
    fn scope_rejects_unspecified_and_accepts_exact_operations() {
        assert!(scope(Some(&PersonalAccessTokenScope::default())).is_err());
        let valid = PersonalAccessTokenScope {
            operations: vec![GitOperation::Discover.into(), GitOperation::Fetch.into()],
            ..Default::default()
        };
        assert!(scope(Some(&valid)).is_ok());
    }

    #[test]
    fn timestamp_rejects_invalid_nanos() {
        assert!(
            timestamp(Some(&buffa_types::google::protobuf::Timestamp {
                seconds: 1,
                nanos: 1_000_000_000,
                ..Default::default()
            }))
            .is_err()
        );
    }
}
