//! Repository RPC composition and shared conversions.

mod create_repository;
mod get_repository;
mod list_repository_instances;

use super::{MediatorAuthenticator, RpcError};
use crate::application::repository::{Page, RepositoryApplication, RepositoryError};
use crate::rpc::MutationReceipts;
use connectrpc::{RequestContext, Router, ServiceRequest, ServiceResult};
use control_plane_postgres::ControlPlanePool as PgPool;
use forge_postgres::PgForgeRepository;
use forge_service::ForgeRepositoryError;
use rpc_proto::{
    connect::hephaestus::repository::v1::{RepositoryService, RepositoryServiceExt},
    messages::hephaestus::{
        common::v1::{OpaqueId, PageRequest},
        repository::v1::{
            CreateRepositoryRequest, CreateRepositoryResponse, GetRepositoryRequest,
            GetRepositoryResponse, ListRepositoryInstancesRequest, ListRepositoryInstancesResponse,
        },
    },
};
use time::OffsetDateTime;
use uuid::Uuid;

const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 200;

pub struct RepositoryRpc {
    application: RepositoryApplication,
    forge: std::sync::Arc<PgForgeRepository>,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
}

impl RepositoryRpc {
    const fn new(
        pool: PgPool,
        forge: std::sync::Arc<PgForgeRepository>,
        authenticator: MediatorAuthenticator,
        receipts: MutationReceipts,
    ) -> Self {
        Self {
            application: RepositoryApplication::new(pool),
            forge,
            authenticator,
            receipts,
        }
    }
}

pub fn register(
    router: Router,
    pool: PgPool,
    forge: std::sync::Arc<PgForgeRepository>,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
) -> Router {
    RepositoryServiceExt::register(
        std::sync::Arc::new(RepositoryRpc::new(pool, forge, authenticator, receipts)),
        router,
    )
}

#[allow(refining_impl_trait)]
impl RepositoryService for RepositoryRpc {
    async fn create_repository(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateRepositoryRequest>,
    ) -> ServiceResult<CreateRepositoryResponse> {
        create_repository::handle(self, ctx, request).await
    }

    async fn get_repository(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetRepositoryRequest>,
    ) -> ServiceResult<GetRepositoryResponse> {
        get_repository::handle(self, ctx, request).await
    }

    async fn list_repository_instances(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListRepositoryInstancesRequest>,
    ) -> ServiceResult<ListRepositoryInstancesResponse> {
        list_repository_instances::handle(self, ctx, request).await
    }
}

fn map_forge_error(error: &ForgeRepositoryError) -> RpcError {
    match error {
        ForgeRepositoryError::AuthorizationDenied => RpcError::PermissionDenied,
        ForgeRepositoryError::AuthorizationUnavailable
        | ForgeRepositoryError::GitStorage(_)
        | ForgeRepositoryError::GitInspection(_)
        | ForgeRepositoryError::Storage(_) => RpcError::Unavailable,
        ForgeRepositoryError::InvalidMetadata(_) => RpcError::InvalidArgument,
        ForgeRepositoryError::RepositoryNotFound(_) => RpcError::NotFound,
        ForgeRepositoryError::InvalidStoredData(_) | ForgeRepositoryError::Serialization(_) => {
            tracing::error!(error = %error, "forge repository creation returned invalid data");
            RpcError::Internal
        }
        _ => {
            tracing::error!(error = %error, "forge repository creation failed");
            RpcError::Internal
        }
    }
}

fn parse_id(value: Option<&OpaqueId>) -> Result<Uuid, RpcError> {
    value
        .ok_or(RpcError::InvalidArgument)?
        .value
        .parse()
        .map_err(|_| RpcError::InvalidArgument)
}

fn parse_page(value: Option<&PageRequest>) -> Result<Page, RpcError> {
    let size = value.map_or(DEFAULT_PAGE_SIZE, |page| {
        if page.page_size == 0 {
            DEFAULT_PAGE_SIZE
        } else {
            page.page_size
        }
    });
    if size > MAX_PAGE_SIZE {
        return Err(RpcError::InvalidArgument);
    }
    let after = value
        .filter(|page| !page.page_token.is_empty())
        .map(|page| page.page_token.parse())
        .transpose()
        .map_err(|_| RpcError::InvalidArgument)?;
    Ok(Page {
        size: i64::from(size),
        after,
    })
}

fn opaque(value: Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}

fn timestamp(value: OffsetDateTime) -> buffa_types::google::protobuf::Timestamp {
    buffa_types::google::protobuf::Timestamp {
        seconds: value.unix_timestamp(),
        nanos: value.nanosecond().cast_signed(),
        ..Default::default()
    }
}

fn map_error(error: RepositoryError) -> RpcError {
    match error {
        RepositoryError::PermissionDenied => RpcError::PermissionDenied,
        RepositoryError::NotFound => RpcError::NotFound,
        RepositoryError::InvalidPage => RpcError::InvalidArgument,
        RepositoryError::Persistence(source) => {
            tracing::error!(error = %source, "repository query failed");
            RpcError::Unavailable
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_PAGE_SIZE, map_forge_error, parse_page, timestamp};
    use crate::rpc::RpcError;
    use forge_service::ForgeRepositoryError;
    use rpc_proto::messages::hephaestus::common::v1::PageRequest;

    #[test]
    fn page_is_bounded_and_cursor_is_a_uuid() {
        assert!(parse_page(None).is_ok());
        assert!(
            parse_page(Some(&PageRequest {
                page_size: MAX_PAGE_SIZE + 1,
                ..Default::default()
            }))
            .is_err()
        );
        assert!(
            parse_page(Some(&PageRequest {
                page_token: String::from("bad"),
                ..Default::default()
            }))
            .is_err()
        );
    }

    #[test]
    fn repository_timestamp_is_constructed_without_json_round_trip() {
        let value = time::OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .expect("valid fixture timestamp")
            + time::Duration::nanoseconds(987_654_321);
        let projected = timestamp(value);
        assert_eq!(projected.seconds, value.unix_timestamp());
        assert_eq!(projected.nanos, 987_654_321);
    }

    #[test]
    fn repository_creation_preserves_authorization_and_validation_categories() {
        assert_eq!(
            map_forge_error(&ForgeRepositoryError::AuthorizationDenied),
            RpcError::PermissionDenied
        );
        assert_eq!(
            map_forge_error(&ForgeRepositoryError::InvalidMetadata("invalid branch")),
            RpcError::InvalidArgument
        );
    }
}
