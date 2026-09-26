//! Connect service implementation for personal access tokens.

use super::super::{
    MediatorAuthenticator, MutationReceipts, RpcError, into_connect_error, request,
};
use super::{
    application_error, identity_receipt, metadata, mutation_identity, scope, timestamp, token_id,
};
use connectrpc::{RequestContext, Response, Router, ServiceRequest, ServiceResult};
use control_plane_postgres::ControlPlanePool as PgPool;
use pat_domain::{PersonalAccessTokenId, PersonalAccessTokenLabel};
use pat_postgres::{
    CreatePersonalAccessToken as CreateCommand, PostgresPersonalAccessTokenService,
    RotatePersonalAccessToken as RotateCommand,
};
use rpc_proto::{
    connect::hephaestus::pat::v1::{PersonalAccessTokenService, PersonalAccessTokenServiceExt},
    messages::hephaestus::{
        common::v1::PageResponse,
        pat::v1::{
            CreatePersonalAccessTokenRequest, CreatePersonalAccessTokenResponse,
            ListPersonalAccessTokensRequest, ListPersonalAccessTokensResponse,
            PersonalAccessTokenValue, RevokePersonalAccessTokenRequest,
            RevokePersonalAccessTokenResponse, RotatePersonalAccessTokenRequest,
            RotatePersonalAccessTokenResponse,
        },
    },
};
use std::str::FromStr;

const DEFAULT_PAGE_SIZE: usize = 50;
const MAX_PAGE_SIZE: usize = 100;

/// RPC implementation for personal access token operations.
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

/// Registers the personal access token service on the Connect router.
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
        let budget = request::RequestBudget::from_transport(&ctx);
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
        let values = request::run_with_budget(&budget, self.application.list(&identity))
            .await
            .map_err(into_connect_error)?
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
        let budget = request::RequestBudget::from_transport(&ctx);
        let request = request.to_owned_message();
        let identity = mutation_identity(
            &ctx,
            &self.authenticator,
            "CreatePersonalAccessToken",
            &request.context,
        )?;
        let issued = request::run_with_budget(
            &budget,
            self.application.create(
                &identity,
                CreateCommand {
                    label: PersonalAccessTokenLabel::parse(request.label)
                        .map_err(|_| into_connect_error(RpcError::InvalidArgument))?,
                    scope: scope(request.scope.as_option())?,
                    expires_at: timestamp(request.expires_at.as_option())?,
                },
            ),
        )
        .await
        .map_err(into_connect_error)?
        .map_err(application_error)?;
        let receipt =
            request::run_with_budget(&budget, identity_receipt(&self.receipts, &identity))
                .await
                .map_err(into_connect_error)??;
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
        let budget = request::RequestBudget::from_transport(&ctx);
        let request = request.to_owned_message();
        let identity = mutation_identity(
            &ctx,
            &self.authenticator,
            "RotatePersonalAccessToken",
            &request.context,
        )?;
        let issued = request::run_with_budget(
            &budget,
            self.application.rotate(
                &identity,
                RotateCommand {
                    token_id: token_id(request.token_id.as_option())?,
                    label: PersonalAccessTokenLabel::parse(request.label)
                        .map_err(|_| into_connect_error(RpcError::InvalidArgument))?,
                    scope: scope(request.scope.as_option())?,
                    expires_at: timestamp(request.expires_at.as_option())?,
                },
            ),
        )
        .await
        .map_err(into_connect_error)?
        .map_err(application_error)?;
        let receipt =
            request::run_with_budget(&budget, identity_receipt(&self.receipts, &identity))
                .await
                .map_err(into_connect_error)??;
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
        let budget = request::RequestBudget::from_transport(&ctx);
        let request = request.to_owned_message();
        let identity = mutation_identity(
            &ctx,
            &self.authenticator,
            "RevokePersonalAccessToken",
            &request.context,
        )?;
        let revoked = request::run_with_budget(
            &budget,
            self.application
                .revoke(&identity, token_id(request.token_id.as_option())?),
        )
        .await
        .map_err(into_connect_error)?
        .map_err(application_error)?;
        let receipt =
            request::run_with_budget(&budget, identity_receipt(&self.receipts, &identity))
                .await
                .map_err(into_connect_error)??;
        Response::ok(RevokePersonalAccessTokenResponse {
            token: metadata(&revoked).into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }
}
